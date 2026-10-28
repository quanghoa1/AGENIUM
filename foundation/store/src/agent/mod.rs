//! Per-agent private memory database (`db/agents/<agent-id>.db`): facts with an exact cosine vector index
//! (sqlite-vec `vec0`), BM25 keyword search (FTS5), hybrid retrieval by reciprocal-rank fusion, the fact-usage log
//! and the resumable conversation store. Only the owning agent's runtime opens this file (decision 2).
//!
//! KNN goes through [`crate::ann`]: exact below `ann_threshold` facts, the usearch sidecar from there (ADR-50). Every
//! vector change is journaled in `ann_journal` in the same transaction.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::ann::{AgentOptions, AnnStatus, Exact, IndexKind, VectorIndex, Vectors};
use crate::db::{Db, DbOptions};
use crate::migrate::{migrate, Migration};
use crate::{backup, StoreError};

mod facts;
mod overlays;
mod portable;
mod search;
mod upkeep;
mod write;

pub use facts::*;
pub use search::{fts_query, fts_search_query};

pub const AGENT_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("../schema/agent_001.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("../schema/agent_002.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("../schema/agent_003.sql"),
    },
    Migration {
        version: 4,
        sql: include_str!("../schema/agent_004.sql"),
    },
    Migration {
        version: 5,
        sql: include_str!("../schema/agent_005.sql"),
    },
    Migration {
        version: 6,
        sql: include_str!("../schema/agent_006.sql"),
    },
];

/// Tables carried by an agent export (in dependency order). `agent_meta`, vectors and the ANN journal are
/// derived or machine-specific.
pub const EXPORT_TABLES: &[&str] = &[
    "fact",
    "entity",
    "entity_alias",
    "entity_merge_log",
    "edge",
    "fact_entity",
    "fact_usage",
    "conversation",
    "skill_overlay",
];

/// Reciprocal rank fusion constant (ADR-51 §hybrid retrieval).
pub const RRF_K: f64 = 60.0;
/// Facts whose decayed weight falls below this are not retrieved (kept, not deleted; decision 35).
pub const HIDE_BELOW_WEIGHT: f64 = 0.2;
/// Half-life of a fact's weight while it is not used (decision 35).
pub const DECAY_HALF_LIFE_DAYS: f64 = 90.0;

pub(crate) fn vec_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Can the vector take part in cosine search? A zero (or non-finite) vector has no direction: sqlite-vec returns a NULL
/// distance for it, and one such row breaks every search of the table. The hashing embedder returns one for text
/// without a word of two letters ("C#"). Such facts and entities are stored without a vector (found by keyword, alias
/// and graph) and such queries skip the vector channel.
/// Since ADR-94 `graph::canonical` keeps `+` and `#` after a word ("c#", "c++"); names and aliases stored before
/// were keyed without them ("c"). Re-keys them once per database (they are rare: the scan matches only names with
/// those characters). Scope values normalised before cannot be recovered ("c" may have been C, C# or C++) and stay.
fn rekey_names(c: &mut Connection) -> Result<(), StoreError> {
    let done: Option<String> = c
        .query_row(
            "SELECT value FROM agent_meta WHERE key = 'canonical'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if done.as_deref() == Some("2") {
        return Ok(());
    }
    let tx = c.transaction()?;
    {
        let rows: Vec<(i64, String, String)> = tx
            .prepare("SELECT id, name, canonical_key FROM entity WHERE name GLOB '*[+#]*'")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?;
        for (id, name, key) in rows {
            let new = crate::graph::canonical(&name);
            if new != key {
                tx.execute(
                    "UPDATE entity SET canonical_key = ?2 WHERE id = ?1",
                    params![id, new],
                )?;
            }
        }
        let rows: Vec<(i64, String, String)> = tx
            .prepare(
                "SELECT entity_id, alias_norm, alias FROM entity_alias WHERE alias GLOB '*[+#]*'",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?;
        for (id, norm, alias) in rows {
            let new = crate::graph::canonical(&alias);
            if new != norm {
                tx.execute(
                    "UPDATE OR IGNORE entity_alias SET alias_norm = ?3 WHERE entity_id = ?1 AND alias_norm = ?2",
                    params![id, norm, new],
                )?;
            }
        }
    }
    tx.execute(
        "INSERT INTO agent_meta(key, value) VALUES ('canonical', '2') ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn usable(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite()) && v.iter().any(|x| *x != 0.0)
}

pub struct AgentStore {
    pub(crate) db: Arc<Db>,
    vectors: Arc<Vectors>,
    dim: usize,
    model_id: String,
}

impl Drop for AgentStore {
    /// Stops a sidecar build and saves the sidecar's unsaved changes.
    fn drop(&mut self) {
        self.vectors.close();
    }
}

impl std::fmt::Debug for AgentStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentStore")
            .field("db", &self.db)
            .field("dim", &self.dim)
            .finish()
    }
}

impl AgentStore {
    /// Opens (creating/migrating) an agent database. The embedding dimension is fixed per database (ADR-47):
    /// opening with another dimension fails; changing the embedder model with the same dimension is allowed and
    /// recorded (vectors from the old model are re-embedded by `reindex`).
    pub fn open(
        path: &Path,
        dim: usize,
        model_id: &str,
        pre_migrate_dir: &Path,
    ) -> Result<AgentStore, StoreError> {
        Self::open_with(
            path,
            dim,
            model_id,
            pre_migrate_dir,
            AgentOptions::default(),
        )
    }

    /// [`AgentStore::open`] with the vector-index options (ADR-50: mode, ANN threshold, sidecar parameters).
    pub fn open_with(
        path: &Path,
        dim: usize,
        model_id: &str,
        pre_migrate_dir: &Path,
        options: AgentOptions,
    ) -> Result<AgentStore, StoreError> {
        let opts = DbOptions {
            synchronous_full: false,
            readers: 2,
            // Address space, not memory: pages are read in only when used. A 1 M-fact database is 1.9 GB, and with
            // the default 1 GiB map its BM25 search took 81 ms p50 instead of 45 (STORE-SCALE §15).
            mmap_bytes: 4 << 30,
            ..DbOptions::default()
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("agent")
            .to_owned();
        let db = Db::open(path, opts, |c| {
            migrate(c, AGENT_MIGRATIONS, |c, from| {
                backup::pre_migrate_snapshot(c, pre_migrate_dir, &format!("agent-{name}"), from)
                    .map(|_| ())
            })?;
            let stored: Option<String> = c
                .query_row(
                    "SELECT value FROM agent_meta WHERE key = 'embedding_dim'",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            match stored {
                Some(d) if d != dim.to_string() => {
                    return Err(StoreError::Invalid(format!(
                        "agent database uses {d}-d embeddings, embedder gives {dim}"
                    )));
                }
                Some(_) => {}
                None => {
                    c.execute(
                        "INSERT INTO agent_meta(key, value) VALUES ('embedding_dim', ?1)",
                        [dim.to_string()],
                    )?;
                }
            }
            c.execute(
                "INSERT INTO agent_meta(key, value) VALUES ('embedder', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [model_id],
            )?;
            c.execute_batch(&format!(
                "CREATE VIRTUAL TABLE IF NOT EXISTS fact_vec USING vec0(embedding float[{dim}] distance_metric=cosine);
                 CREATE VIRTUAL TABLE IF NOT EXISTS entity_vec USING vec0(embedding float[{dim}] distance_metric=cosine);"
            ))?;
            rekey_names(c)?;
            Ok(())
        })?;
        let db = Arc::new(db);
        let vectors = Vectors::open(db.clone(), dim, options)?;
        Ok(AgentStore {
            db,
            vectors,
            dim,
            model_id: model_id.to_owned(),
        })
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub(crate) fn check_vec(&self, v: &[f32]) -> Result<(), StoreError> {
        if v.len() != self.dim {
            return Err(StoreError::Invalid(format!(
                "vector has {} dims, expected {}",
                v.len(),
                self.dim
            )));
        }
        Ok(())
    }
}
