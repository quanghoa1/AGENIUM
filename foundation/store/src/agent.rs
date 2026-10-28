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

pub const AGENT_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("schema/agent_001.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("schema/agent_002.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("schema/agent_003.sql"),
    },
    Migration {
        version: 4,
        sql: include_str!("schema/agent_004.sql"),
    },
    Migration {
        version: 5,
        sql: include_str!("schema/agent_005.sql"),
    },
    Migration {
        version: 6,
        sql: include_str!("schema/agent_006.sql"),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FactKind {
    Fact,
    Preference,
    Lesson,
}

impl FactKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::Lesson => "lesson",
        }
    }
    fn parse(s: &str) -> Result<Self, StoreError> {
        match s {
            "fact" => Ok(Self::Fact),
            "preference" => Ok(Self::Preference),
            "lesson" => Ok(Self::Lesson),
            o => Err(StoreError::Corrupt(format!("fact kind {o:?}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewFact {
    pub text: String,
    pub kind: FactKind,
    pub project: Option<String>,
    pub scope: serde_json::Value,
    pub cue: Option<String>,
    pub evidence: Option<String>,
    pub source: String,
    pub untrusted: bool,
    pub confidence: f64,
}

impl NewFact {
    pub fn fact(text: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: FactKind::Fact,
            project: None,
            scope: serde_json::json!({}),
            cue: None,
            evidence: None,
            source: source.into(),
            untrusted: false,
            confidence: 0.5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    pub id: i64,
    pub text: String,
    pub kind: FactKind,
    pub project: Option<String>,
    pub scope: serde_json::Value,
    pub cue: Option<String>,
    pub evidence: Option<String>,
    pub source: String,
    pub untrusted: bool,
    pub confidence: f64,
    pub weight: f64,
    pub model_id: Option<String>,
    pub superseded_by: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
    pub used_at: Option<String>,
    /// Contexts in which the fact (a lesson) was used and the work held (decision 35).
    #[serde(default)]
    pub validated: Vec<String>,
}

impl Fact {
    pub fn is_live(&self) -> bool {
        self.superseded_by.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    Vector,
    Keyword,
    Hybrid,
    Graph,
    Recent,
    /// A rule of the work's context: a proven lesson whose scope matches it (decision 35), shown whatever the
    /// text channels find.
    Rule,
}

impl Via {
    pub fn as_str(self) -> &'static str {
        match self {
            Via::Vector => "vector",
            Via::Keyword => "keyword",
            Via::Hybrid => "hybrid",
            Via::Graph => "graph",
            Via::Recent => "recent",
            Via::Rule => "rule",
        }
    }
}

/// An agent's copy-on-write version of a skill (decision 35).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlayRow {
    pub skill: String,
    pub base_version: String,
    /// Unified diff from the base SKILL.md to the overlay's.
    pub patch: String,
    /// One entry per update: when, why, the lesson behind it.
    pub changelog: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    pub fact: Fact,
    pub score: f64,
    pub via: Via,
}

const FACT_COLS: &str = "id, text, kind, project, scope, cue, evidence, source, untrusted, confidence, weight, model_id, \
    superseded_by, created_at, updated_at, used_at, validated";

fn map_fact(r: &rusqlite::Row<'_>) -> rusqlite::Result<(Fact, String, String)> {
    let validated: String = r.get(16)?;
    Ok((
        Fact {
            id: r.get(0)?,
            text: r.get(1)?,
            kind: FactKind::Fact,
            project: r.get(3)?,
            scope: serde_json::Value::Null,
            cue: r.get(5)?,
            evidence: r.get(6)?,
            source: r.get(7)?,
            untrusted: r.get(8)?,
            confidence: r.get(9)?,
            weight: r.get(10)?,
            model_id: r.get(11)?,
            superseded_by: r.get(12)?,
            created_at: r.get(13)?,
            updated_at: r.get(14)?,
            used_at: r.get(15)?,
            validated: serde_json::from_str(&validated).unwrap_or_default(),
        },
        r.get(2)?,
        r.get(4)?,
    ))
}

fn decode_fact(v: (Fact, String, String)) -> Result<Fact, StoreError> {
    let (mut f, kind, scope) = v;
    f.kind = FactKind::parse(&kind)?;
    f.scope = crate::core::from_json(&scope)?;
    Ok(f)
}

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

/// Turns free text into a safe FTS5 query: every word token quoted, OR-ed (BM25 ranks by overlap).
pub fn fts_query(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2)
        .take(32)
        .map(|w| format!("\"{}\"", w.to_lowercase()))
        .collect();
    (!words.is_empty()).then(|| words.join(" OR "))
}

/// English words so common that BM25 gives them almost no weight, yet matching them makes a search touch most of the
/// index (re-measured 2026-10-25: queries keeping them cost several times more at 100 k facts).
const STOPWORDS: &[&str] = &[
    "a", "about", "after", "all", "also", "an", "and", "any", "are", "as", "at", "be", "been",
    "before", "but", "by", "can", "could", "did", "do", "does", "for", "from", "had", "has",
    "have", "he", "her", "his", "how", "i", "if", "in", "into", "is", "it", "its", "may", "me",
    "more", "most", "my", "no", "not", "of", "on", "one", "only", "or", "other", "our", "out",
    "over", "she", "should", "so", "some", "such", "than", "that", "the", "their", "them", "then",
    "there", "these", "they", "this", "those", "through", "to", "up", "us", "very", "was", "we",
    "were", "what", "when", "where", "which", "while", "who", "will", "with", "would", "you",
    "your",
];

/// The FTS5 query for a fact search: like [`fts_query`] without stopwords. `None` when only stopwords remain (the
/// vector channel still answers).
pub fn fts_search_query(text: &str) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 2 && !STOPWORDS.contains(&w.as_str()))
        .take(32)
        .map(|w| format!("\"{w}\""))
        .collect();
    (!words.is_empty()).then(|| words.join(" OR "))
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

    fn insert_fact(
        &self,
        tx: &Transaction<'_>,
        f: &NewFact,
        vector: Option<&[f32]>,
    ) -> Result<i64, StoreError> {
        tx.execute(
            "INSERT INTO fact(text, kind, project, scope, cue, evidence, source, untrusted, confidence, model_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                f.text,
                f.kind.as_str(),
                f.project,
                f.scope.to_string(),
                f.cue,
                f.evidence,
                f.source,
                f.untrusted,
                f.confidence,
                vector.map(|_| self.model_id.as_str())
            ],
        )?;
        let id = tx.last_insert_rowid();
        if let Some(v) = vector.filter(|v| usable(v)) {
            tx.execute(
                "INSERT INTO fact_vec(rowid, embedding) VALUES (?1, ?2)",
                params![id, vec_blob(v)],
            )?;
            tx.execute("INSERT INTO ann_journal(op, fact_id) VALUES (1, ?1)", [id])?;
        }
        Ok(id)
    }

    fn drop_vector(tx: &Transaction<'_>, id: i64) -> Result<(), StoreError> {
        if tx.execute("DELETE FROM fact_vec WHERE rowid = ?1", [id])? > 0 {
            tx.execute("INSERT INTO ann_journal(op, fact_id) VALUES (2, ?1)", [id])?;
        }
        Ok(())
    }

    /// ADD.
    pub fn add(&self, f: &NewFact, vector: Option<&[f32]>) -> Result<i64, StoreError> {
        if let Some(v) = vector {
            self.check_vec(v)?;
        }
        if f.text.trim().is_empty() {
            return Err(StoreError::Invalid("fact text is empty".into()));
        }
        let id = self.db.write(|tx| self.insert_fact(tx, f, vector))?;
        self.vectors.after_write();
        Ok(id)
    }

    /// UPDATE: a new fact supersedes `old` (history kept, decision 35). Returns the new id.
    pub fn supersede(
        &self,
        old: i64,
        f: &NewFact,
        vector: Option<&[f32]>,
    ) -> Result<i64, StoreError> {
        if let Some(v) = vector {
            self.check_vec(v)?;
        }
        self.db.write(|tx| {
            let live: Option<Option<i64>> =
                tx.query_row("SELECT superseded_by FROM fact WHERE id = ?1", [old], |r| r.get(0)).optional()?;
            match live {
                None => return Err(StoreError::NotFound(format!("fact {old}"))),
                Some(Some(_)) => return Err(StoreError::Conflict(format!("fact {old} is already superseded"))),
                Some(None) => {}
            }
            let id = self.insert_fact(tx, f, vector)?;
            tx.execute(
                "UPDATE fact SET superseded_by = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?1",
                params![old, id],
            )?;
            Self::drop_vector(tx, old)?;
            Ok(id)
        })
        .inspect(|_| self.vectors.after_write())
    }

    /// DELETE: retires a fact (self-superseded tombstone; the text stays for audit, it is never retrieved).
    pub fn retire(&self, id: i64) -> Result<(), StoreError> {
        self.db.write(|tx| {
            let n = tx.execute(
                "UPDATE fact SET superseded_by = id, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                 WHERE id = ?1 AND superseded_by IS NULL",
                [id],
            )?;
            if n == 0 {
                return Err(StoreError::NotFound(format!("live fact {id}")));
            }
            Self::drop_vector(tx, id)
        })?;
        self.vectors.after_write();
        Ok(())
    }

    pub fn get(&self, id: i64) -> Result<Fact, StoreError> {
        self.db
            .read(|c| {
                c.query_row(
                    &format!("SELECT {FACT_COLS} FROM fact WHERE id = ?1"),
                    [id],
                    map_fact,
                )
                .optional()
                .map_err(StoreError::from)
            })?
            .map(decode_fact)
            .transpose()?
            .ok_or_else(|| StoreError::NotFound(format!("fact {id}")))
    }

    pub(crate) fn facts_by_ids(
        c: &Connection,
        ids: &[i64],
    ) -> Result<HashMap<i64, Fact>, StoreError> {
        let mut out = HashMap::new();
        let mut st = c.prepare_cached(&format!("SELECT {FACT_COLS} FROM fact WHERE id = ?1"))?;
        for id in ids {
            if let Some(v) = st.query_row([id], map_fact).optional()? {
                out.insert(*id, decode_fact(v)?);
            }
        }
        Ok(out)
    }

    /// Live facts, newest first.
    pub fn live(&self, limit: usize) -> Result<Vec<Fact>, StoreError> {
        self.db.read(|c| {
            let mut st =
                c.prepare_cached(&format!("SELECT {FACT_COLS} FROM fact WHERE superseded_by IS NULL ORDER BY id DESC LIMIT ?1"))?;
            let rows = st.query_map([limit as i64], map_fact)?;
            rows.map(|r| decode_fact(r?)).collect()
        })
    }

    /// Live, trusted lessons whose scope holds in `ctx`: the lesson has a scope, and each of its keys has the same
    /// value in `ctx` (decision 35: the rules of that context). The most proven first: held in more contexts, then
    /// more confident, then newer.
    pub fn rules_in(&self, ctx: &serde_json::Value, limit: usize) -> Result<Vec<Fact>, StoreError> {
        let ctx = crate::graph::normalize_scope(ctx);
        let Some(c) = ctx.as_object().filter(|c| !c.is_empty()) else {
            return Ok(vec![]);
        };
        let mut out: Vec<Fact> = self.db.read(|db| {
            let mut st = db.prepare_cached(&format!(
                "SELECT {FACT_COLS} FROM fact WHERE superseded_by IS NULL AND kind = 'lesson' AND untrusted = 0"
            ))?;
            let rows = st.query_map([], map_fact)?;
            rows.map(|r| decode_fact(r?)).collect::<Result<Vec<_>, StoreError>>()
        })?;
        out.retain(|f| {
            f.weight >= HIDE_BELOW_WEIGHT
                && crate::graph::normalize_scope(&f.scope)
                    .as_object()
                    .is_some_and(|o| !o.is_empty() && o.iter().all(|(k, v)| c.get(k) == Some(v)))
        });
        out.sort_by(|a, b| {
            b.validated
                .len()
                .cmp(&a.validated.len())
                .then(b.confidence.total_cmp(&a.confidence))
                .then(b.id.cmp(&a.id))
        });
        out.truncate(limit);
        Ok(out)
    }

    /// Maintenance tick for this agent's database: passive checkpoint, and the WAL gate past the limit. Returns the WAL
    /// size afterwards. (Writes also run the WAL gate themselves; the tick catches databases that only read.)
    /// Also saves unsaved sidecar changes.
    pub fn maintain(&self) -> Result<u64, StoreError> {
        self.vectors.maintain()?;
        self.db.maintain()
    }

    /// Saves the ANN sidecar now (it is also saved every `save_every` changes, by `maintain` and on close).
    pub fn flush(&self) -> Result<(), StoreError> {
        self.vectors.save()
    }

    /// The vector-index options this store was opened with.
    pub fn index_options(&self) -> &AgentOptions {
        self.vectors.options()
    }

    /// Which index serves KNN now, and the sidecar's state.
    pub fn index_status(&self) -> AnnStatus {
        self.vectors.status()
    }

    /// Waits for a sidecar build to finish (benchmarks, tests). `false` on timeout.
    pub fn wait_for_index(&self, timeout: Duration) -> bool {
        self.vectors.wait_ready(timeout)
    }

    /// Exact KNN whatever the active index (recall measurement, tests).
    pub fn knn_exact(&self, query: &[f32], k: usize) -> Result<Vec<(i64, f64)>, StoreError> {
        self.check_vec(query)?;
        if k == 0 || !usable(query) {
            return Ok(vec![]);
        }
        Exact(&self.db).knn(query, k)
    }

    /// Is there any live fact? (Stops at the first one; `count_live` walks them all, ~20 ms at 1 M facts.)
    pub fn has_live(&self) -> Result<bool, StoreError> {
        self.db.read(|c| {
            Ok(c.query_row(
                "SELECT EXISTS (SELECT 1 FROM fact WHERE superseded_by IS NULL)",
                [],
                |r| r.get(0),
            )?)
        })
    }

    pub fn count_live(&self) -> Result<u64, StoreError> {
        self.db.read(|c| {
            Ok(c.query_row(
                "SELECT count(*) FROM fact WHERE superseded_by IS NULL",
                [],
                |r| r.get::<_, i64>(0),
            )? as u64)
        })
    }

    /// Cosine similarity of the query to each given fact that has a vector (facts without one are absent).
    pub fn similarity_to(
        &self,
        query: &[f32],
        ids: &[i64],
    ) -> Result<HashMap<i64, f64>, StoreError> {
        self.check_vec(query)?;
        let mut out = HashMap::new();
        if ids.is_empty() || !usable(query) {
            return Ok(out);
        }
        let qn = query
            .iter()
            .map(|x| (*x as f64).powi(2))
            .sum::<f64>()
            .sqrt();
        self.db.read(|c| {
            let mut st = c.prepare_cached("SELECT embedding FROM fact_vec WHERE rowid = ?1")?;
            for id in ids {
                let blob: Option<Vec<u8>> = st.query_row([id], |r| r.get(0)).optional()?;
                let Some(blob) = blob else { continue };
                let v = blob
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b) as f64);
                let (mut dot, mut vn) = (0.0, 0.0);
                for (x, q) in v.zip(query) {
                    dot += x * *q as f64;
                    vn += x * x;
                }
                if vn > 0.0 && qn > 0.0 {
                    out.insert(*id, dot / (vn.sqrt() * qn));
                }
            }
            Ok(out)
        })
    }

    /// Cosine KNN over live facts: `(fact id, distance)` ascending, through the active index (the ANN sidecar from
    /// `ann_threshold` facts, else exact). A query without a direction finds nothing.
    pub fn knn(&self, query: &[f32], k: usize) -> Result<Vec<(i64, f64)>, StoreError> {
        self.check_vec(query)?;
        if k == 0 || !usable(query) {
            return Ok(vec![]);
        }
        let rerank = self.vectors.options().ann.rerank;
        match self.vectors.knn(query, if rerank { k * 4 } else { k }) {
            Some(hits) if rerank => {
                let ids: Vec<i64> = hits.iter().map(|h| h.0).collect();
                let sim = self.similarity_to(query, &ids)?;
                let mut out: Vec<(i64, f64)> =
                    sim.into_iter().map(|(id, s)| (id, 1.0 - s)).collect();
                out.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
                out.truncate(k);
                Ok(out)
            }
            Some(hits) => Ok(hits),
            None => Exact(&self.db).knn(query, k),
        }
    }

    /// The index serving KNN now.
    pub fn index_kind(&self) -> IndexKind {
        self.index_status().active
    }

    /// BM25 keyword search over live facts: `(fact id, bm25)` best first.
    pub fn keyword(&self, text: &str, k: usize) -> Result<Vec<(i64, f64)>, StoreError> {
        let Some(q) = fts_search_query(text) else {
            return Ok(vec![]);
        };
        self.db.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT f.id, bm25(fact_fts) AS s FROM fact_fts JOIN fact f ON f.id = fact_fts.rowid
                 WHERE fact_fts MATCH ?1 AND f.superseded_by IS NULL ORDER BY s LIMIT ?2",
            )?;
            let rows = st.query_map(params![q, k as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }

    /// Hybrid retrieval: vector KNN + BM25, fused by reciprocal rank (k = 60), hidden facts filtered.
    /// `project` boosts facts of the current project (decision 25) by ordering ties.
    pub fn hybrid(
        &self,
        text: &str,
        query: Option<&[f32]>,
        k: usize,
    ) -> Result<Vec<Hit>, StoreError> {
        let pool = (k * 4).max(20);
        let vec_hits = match query {
            Some(q) => self.knn(q, pool)?,
            None => vec![],
        };
        let kw_hits = self.keyword(text, pool)?;
        let mut score: HashMap<i64, (f64, bool, bool)> = HashMap::new();
        for (rank, (id, _)) in vec_hits.iter().enumerate() {
            let e = score.entry(*id).or_default();
            e.0 += 1.0 / (RRF_K + rank as f64 + 1.0);
            e.1 = true;
        }
        for (rank, (id, _)) in kw_hits.iter().enumerate() {
            let e = score.entry(*id).or_default();
            e.0 += 1.0 / (RRF_K + rank as f64 + 1.0);
            e.2 = true;
        }
        let ids: Vec<i64> = score.keys().copied().collect();
        let facts = self.db.read(|c| Self::facts_by_ids(c, &ids))?;
        let mut hits: Vec<Hit> = score
            .into_iter()
            .filter_map(|(id, (s, v, kw))| {
                let f = facts.get(&id)?.clone();
                (f.is_live() && f.weight >= HIDE_BELOW_WEIGHT).then_some(Hit {
                    score: s * f.weight,
                    via: match (v, kw) {
                        (true, true) => Via::Hybrid,
                        (true, false) => Via::Vector,
                        _ => Via::Keyword,
                    },
                    fact: f,
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(b.fact.id.cmp(&a.fact.id)));
        hits.truncate(k);
        Ok(hits)
    }

    /// Logs facts injected into a work order (M1-11) and touches `used_at`. A fact logged twice for one run keeps its
    /// first row; any other failure is an error (`INSERT OR IGNORE` also swallowed CHECK failures: rule-injected
    /// lessons were dropped without a word, see agent_004.sql).
    pub fn record_usage(
        &self,
        run: &str,
        ticket: Option<&str>,
        used: &[(i64, u32, Via)],
    ) -> Result<(), StoreError> {
        let rows: Vec<(i64, u32, Via, bool)> =
            used.iter().map(|(f, r, v)| (*f, *r, *v, false)).collect();
        self.record_shown(run, ticket, &rows)
    }

    /// [`Self::record_usage`] with, per fact, whether it was shown as a rule of the work's context (a lesson whose
    /// scope matched exactly, ADR-118).
    pub fn record_shown(
        &self,
        run: &str,
        ticket: Option<&str>,
        used: &[(i64, u32, Via, bool)],
    ) -> Result<(), StoreError> {
        self.db.write(|tx| {
            for (fact, rank, via, as_rule) in used {
                tx.execute(
                    "INSERT INTO fact_usage(fact, ticket, run, rank, via, as_rule) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(run, fact) DO NOTHING",
                    params![fact, ticket, run, *rank as i64, via.as_str(), *as_rule as i64],
                )?;
                tx.execute("UPDATE fact SET used_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?1", [fact])?;
            }
            Ok(())
        })
    }

    /// Records that a fact held in `context` (`project:<id>` / `ticket:<key>`); returns in how many distinct contexts
    /// it has held (decision 35: two or more make a lesson a skill-update candidate).
    pub fn validate_in(&self, fact: i64, context: &str) -> Result<usize, StoreError> {
        self.db.write(|tx| {
            let raw: String =
                tx.query_row("SELECT validated FROM fact WHERE id = ?1", [fact], |r| {
                    r.get(0)
                })?;
            let mut v: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
            if !v.iter().any(|c| c == context) {
                v.push(context.to_owned());
                tx.execute(
                    "UPDATE fact SET validated = ?2 WHERE id = ?1",
                    params![
                        fact,
                        serde_json::to_string(&v).unwrap_or_else(|_| "[]".into())
                    ],
                )?;
            }
            Ok(v.len())
        })
    }

    /// Forgetting (decision 35): the weight of every live fact halves for each `half_life_days` it went unused (since
    /// it was last used, else since it was made); below [`HIDE_BELOW_WEIGHT`] it is no longer retrieved but kept.
    /// Returns how many weights changed.
    pub fn decay(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        half_life_days: f64,
    ) -> Result<u64, StoreError> {
        self.db.write(|tx| {
            let rows: Vec<(i64, String, Option<String>, f64)> = {
                let mut st = tx.prepare(
                    "SELECT id, created_at, used_at, weight FROM fact WHERE superseded_by IS NULL",
                )?;
                let rows = st
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            let mut n = 0;
            for (id, created, used, old) in rows {
                let since = used.unwrap_or(created);
                let Ok(at) = chrono::DateTime::parse_from_rfc3339(&since) else {
                    continue;
                };
                let days =
                    (now - at.with_timezone(&chrono::Utc)).num_seconds().max(0) as f64 / 86_400.0;
                let w = 0.5f64.powf(days / half_life_days.max(1.0));
                if (w - old).abs() > 1e-6 {
                    tx.execute("UPDATE fact SET weight = ?2 WHERE id = ?1", params![id, w])?;
                    n += 1;
                }
            }
            Ok(n)
        })
    }

    /// Consolidation (decision 35): live facts that say the same thing (same kind, scope, project, trust and text up
    /// to case and spacing) become one: the most confident (then the newest) stays, the others are superseded by it
    /// and keep their history; the contexts they were validated in move to the one that stays. Trust and project are
    /// part of the key: an untrusted copy (made from tool output or another agent's words) never replaces a trusted
    /// fact, and a fact learned in one project is not folded into another's. Returns how many were folded.
    pub fn consolidate(&self) -> Result<u64, StoreError> {
        // (kind, scope, project, untrusted, normalised text) and (id, text, kind, scope, project, untrusted,
        // confidence, validated).
        type Key = (String, String, Option<String>, bool, String);
        type Row = (
            i64,
            String,
            String,
            String,
            Option<String>,
            bool,
            f64,
            String,
        );
        self.db.write(|tx| {
            let rows: Vec<Row> = {
                let mut st = tx.prepare(
                    "SELECT id, text, kind, scope, project, untrusted, confidence, validated FROM fact
                     WHERE superseded_by IS NULL AND id IS NOT NULL",
                )?;
                let rows = st
                    .query_map([], |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                            r.get(7)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            let norm = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
            let mut groups: std::collections::BTreeMap<Key, Vec<(i64, f64, String)>> =
                std::collections::BTreeMap::new();
            for (id, text, kind, scope, project, untrusted, conf, validated) in rows {
                groups
                    .entry((kind, scope, project, untrusted, norm(&text)))
                    .or_default()
                    .push((id, conf, validated));
            }
            let mut n = 0;
            for (_, mut ids) in groups.into_iter().filter(|(_, v)| v.len() > 1) {
                ids.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.0.cmp(&a.0)));
                let keep = ids[0].0;
                let contexts = |raw: &str| serde_json::from_str::<Vec<String>>(raw).unwrap_or_default();
                let mut validated = contexts(&ids[0].2);
                for (id, _, other) in &ids[1..] {
                    for c in contexts(other) {
                        if !validated.contains(&c) {
                            validated.push(c);
                        }
                    }
                    tx.execute(
                        "UPDATE fact SET superseded_by = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?1",
                        params![id, keep],
                    )?;
                    Self::drop_vector(tx, *id)?;
                    n += 1;
                }
                if validated != contexts(&ids[0].2) {
                    tx.execute(
                        "UPDATE fact SET validated = ?2 WHERE id = ?1",
                        params![keep, serde_json::to_string(&validated)?],
                    )?;
                }
            }
            Ok(n)
        })
    }

    /// Forgetting and consolidation now (decision 35): `(weights changed, facts folded)`.
    pub fn tidy(&self) -> Result<(u64, u64), StoreError> {
        Ok((
            self.decay(chrono::Utc::now(), DECAY_HALF_LIFE_DAYS)?,
            self.consolidate()?,
        ))
    }

    /// Records an update of the agent's overlay of `skill` (decision 35): the base version it starts from, the
    /// cumulative patch base -> overlay, and one changelog entry appended.
    pub fn put_overlay(
        &self,
        skill: &str,
        base_version: &str,
        patch: &str,
        entry: serde_json::Value,
    ) -> Result<(), StoreError> {
        self.db.write(|tx| {
            let old: Option<String> = tx
                .query_row("SELECT changelog FROM skill_overlay WHERE skill = ?1", [skill], |r| r.get(0))
                .optional()?;
            let mut log: Vec<serde_json::Value> =
                old.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            log.push(entry);
            tx.execute(
                "INSERT INTO skill_overlay(skill, base_version, patch, changelog) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(skill) DO UPDATE SET base_version = excluded.base_version, patch = excluded.patch,
                     changelog = excluded.changelog",
                params![skill, base_version, patch, serde_json::to_string(&log).unwrap_or_else(|_| "[]".into())],
            )?;
            Ok(())
        })
    }

    /// The agent's skill overlays.
    pub fn overlays(&self) -> Result<Vec<OverlayRow>, StoreError> {
        self.db.read(|c| {
            let mut st = c.prepare(
                "SELECT skill, base_version, patch, changelog FROM skill_overlay ORDER BY skill",
            )?;
            let rows = st
                .query_map([], |r| {
                    let log: String = r.get(3)?;
                    Ok(OverlayRow {
                        skill: r.get(0)?,
                        base_version: r.get(1)?,
                        patch: r.get(2)?,
                        changelog: serde_json::from_str(&log).unwrap_or_default(),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn remove_overlay(&self, skill: &str) -> Result<(), StoreError> {
        self.db.write(|tx| {
            tx.execute("DELETE FROM skill_overlay WHERE skill = ?1", [skill])?;
            Ok(())
        })
    }

    /// Lessons that were in play in `run` (ADR-118): shown as a rule of its context and found by the work's own words
    /// (a text or graph channel), not only put in front of every run of the project.
    pub fn lessons_in_play(
        &self,
        run: &str,
    ) -> Result<std::collections::BTreeSet<i64>, StoreError> {
        self.db.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT fact FROM fact_usage WHERE run = ?1 AND as_rule = 1 AND via NOT IN ('rule', 'recent')",
            )?;
            let rows = st.query_map([run], |r| r.get::<_, i64>(0))?;
            rows.collect::<Result<_, _>>().map_err(Into::into)
        })
    }

    pub fn usage_for_run(&self, run: &str) -> Result<Vec<(i64, u32, String)>, StoreError> {
        self.db.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT fact, rank, via FROM fact_usage WHERE run = ?1 ORDER BY rank",
            )?;
            let rows = st.query_map([run], |r| {
                Ok((r.get(0)?, r.get::<_, i64>(1)? as u32, r.get(2)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }

    /// Live facts whose vector is missing or comes from another embedder than `model_id`.
    pub fn stale_vectors(&self, model_id: &str) -> Result<u64, StoreError> {
        self.db.read(|c| -> Result<u64, StoreError> {
            Ok(c.query_row(
                "SELECT count(*) FROM fact WHERE superseded_by IS NULL AND (model_id IS NULL OR model_id <> ?1)",
                [model_id],
                |r| r.get::<_, i64>(0),
            )? as u64)
        })
    }

    /// Replaces the vectors of every live fact (embedder change with the same dimension).
    pub fn reindex(
        &self,
        embed: impl Fn(&[String]) -> Result<Vec<Vec<f32>>, String>,
    ) -> Result<u64, StoreError> {
        let facts = self.live(usize::MAX >> 1)?;
        let texts: Vec<String> = facts.iter().map(|f| f.text.clone()).collect();
        let vecs = embed(&texts).map_err(StoreError::Invalid)?;
        if vecs.len() != facts.len() {
            return Err(StoreError::Invalid(
                "embedder returned a different number of vectors".into(),
            ));
        }
        for v in &vecs {
            self.check_vec(v)?;
        }
        self.db
            .write(|tx| {
                for (f, v) in facts.iter().zip(&vecs) {
                    let had = tx.execute("DELETE FROM fact_vec WHERE rowid = ?1", [f.id])? > 0;
                    tx.execute(
                        "UPDATE fact SET model_id = ?2 WHERE id = ?1",
                        params![f.id, self.model_id],
                    )?;
                    if !usable(v) {
                        if had {
                            tx.execute(
                                "INSERT INTO ann_journal(op, fact_id) VALUES (2, ?1)",
                                [f.id],
                            )?;
                        }
                        continue;
                    }
                    tx.execute(
                        "INSERT INTO fact_vec(rowid, embedding) VALUES (?1, ?2)",
                        params![f.id, vec_blob(v)],
                    )?;
                    tx.execute(
                        "INSERT INTO ann_journal(op, fact_id) VALUES (1, ?1)",
                        [f.id],
                    )?;
                }
                Ok(facts.len() as u64)
            })
            .inspect(|_| self.vectors.after_write())
    }

    // ------------------------------------------------------------ conversation (resumable run context)

    pub fn append_message(
        &self,
        session: &str,
        message: &serde_json::Value,
    ) -> Result<u32, StoreError> {
        self.db.write(|tx| {
            let n: i64 = tx.query_row(
                "SELECT COALESCE(MAX(n), 0) + 1 FROM conversation WHERE session = ?1",
                [session],
                |r| r.get(0),
            )?;
            tx.execute(
                "INSERT INTO conversation(session, n, message) VALUES (?1, ?2, ?3)",
                params![session, n, message.to_string()],
            )?;
            Ok(n as u32)
        })
    }

    pub fn load_messages(&self, session: &str) -> Result<Vec<serde_json::Value>, StoreError> {
        self.db.read(|c| {
            let mut st =
                c.prepare_cached("SELECT message FROM conversation WHERE session = ?1 ORDER BY n")?;
            let rows = st.query_map([session], |r| r.get::<_, String>(0))?;
            rows.map(|s| crate::core::from_json(&s?)).collect()
        })
    }

    // ------------------------------------------------------------ export / import (M15-5)

    /// Writes the agent's memory as JSONL lines: per table `{"kind":"table","table":..,"cols":[..]}` followed by
    /// `{"kind":"row","v":[..]}` lines (rowid order, values typed as in incremental backups). Vectors are not
    /// exported: text is the source of truth and the importer re-embeds.
    pub fn export_jsonl(&self, w: &mut dyn std::io::Write) -> Result<u64, StoreError> {
        self.db.read(|c| -> Result<u64, StoreError> {
            c.execute_batch("BEGIN")?;
            let r = (|| -> Result<u64, StoreError> {
                let mut n = 0u64;
                for t in EXPORT_TABLES {
                    let order = backup::row_order(c, t)?;
                    let mut st = c.prepare(&format!("SELECT * FROM {t} ORDER BY {order}"))?;
                    let cols: Vec<String> =
                        st.column_names().iter().map(|s| s.to_string()).collect();
                    let header = serde_json::json!({ "kind": "table", "table": t, "cols": cols });
                    writeln!(w, "{header}").map_err(|e| StoreError::Invalid(e.to_string()))?;
                    let k = st.column_count();
                    let mut rows = st.query([])?;
                    while let Some(r) = rows.next()? {
                        let vals: Vec<serde_json::Value> = (0..k)
                            .map(|i| r.get_ref(i).map(backup::value_to_json))
                            .collect::<Result<_, _>>()?;
                        let line = serde_json::json!({ "kind": "row", "v": vals });
                        writeln!(w, "{line}").map_err(|e| StoreError::Invalid(e.to_string()))?;
                        n += 1;
                    }
                }
                Ok(n)
            })();
            c.execute_batch("COMMIT")?;
            r
        })
    }

    /// Loads lines written by `export_jsonl` into this (empty) database in one transaction; fact ids are kept so
    /// `superseded_by` and usage rows stay linked. Returns the number of rows. Call `reindex` afterwards.
    pub fn import_jsonl(&self, lines: &[serde_json::Value]) -> Result<u64, StoreError> {
        let existing: i64 = self.db.read(|c| -> Result<i64, StoreError> {
            Ok(c.query_row(
                "SELECT (SELECT count(*) FROM fact) + (SELECT count(*) FROM conversation)",
                [],
                |r| r.get(0),
            )?)
        })?;
        if existing > 0 {
            return Err(StoreError::Invalid(
                "the agent database is not empty; import needs a new agent".into(),
            ));
        }
        self.db.write(|tx| {
            tx.execute_batch("PRAGMA defer_foreign_keys = ON")?;
            let mut sql: Option<String> = None;
            let mut n = 0u64;
            for l in lines {
                match l["kind"].as_str() {
                    Some("table") => {
                        let t = l["table"].as_str().unwrap_or_default();
                        if !EXPORT_TABLES.contains(&t) {
                            return Err(StoreError::Invalid(format!("unexpected table {t:?}")));
                        }
                        let cols: Vec<String> = l["cols"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|c| c.as_str())
                            .map(|c| {
                                if c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
                                    Ok(c.to_owned())
                                } else {
                                    Err(StoreError::Invalid(format!("bad column {c:?}")))
                                }
                            })
                            .collect::<Result<_, _>>()?;
                        sql = Some(format!(
                            "INSERT INTO {t} ({}) VALUES ({})",
                            cols.join(", "),
                            vec!["?"; cols.len()].join(", ")
                        ));
                    }
                    Some("row") => {
                        let Some(q) = &sql else {
                            return Err(StoreError::Invalid("row before its table header".into()));
                        };
                        let vals: Vec<rusqlite::types::Value> = l["v"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(backup::json_to_value)
                            .collect::<Result<_, _>>()?;
                        tx.execute(q, rusqlite::params_from_iter(vals))?;
                        n += 1;
                    }
                    _ => {}
                }
            }
            Ok(n)
        })
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.db.read(|c| {
            Ok(
                c.query_row("SELECT value FROM agent_meta WHERE key = ?1", [key], |r| {
                    r.get(0)
                })
                .optional()?,
            )
        })
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO agent_meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
            Ok(())
        })
    }
}
