//! Vector indexes over an agent's fact vectors (ADR-50): the exact sqlite-vec scan, always present and the source of
//! truth, and the usearch HNSW sidecar `<db>.ann` (+ `<db>.ann.json`) that replaces it from `ann_threshold` facts.
//!
//! The sidecar is derived data and must never serve a wrong result. Every change to `fact_vec` appends to
//! `ann_journal` in the same transaction; the sidecar records the journal position it covers (`applied_seq`) and the
//! `(count, sum(id))` of its keys. Loading checks file length and crc32, that the journal still holds every entry past
//! `applied_seq`, replays them, and compares `(count, sum)` with `fact_vec`; any mismatch or load error discards the
//! sidecar and rebuilds it from `fact_vec` in the background while exact search serves. Replay does not trust the
//! journal's order: each touched fact is set to what `fact_vec` holds now (present or absent), so it is idempotent.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use crate::agent::vec_blob;
use crate::db::Db;
use crate::StoreError;

mod rows;
mod sidecar;
mod vectors;

use rows::*;
pub use sidecar::*;
pub use vectors::*;

/// Facts per agent from which `IndexMode::Auto` builds and uses the sidecar (ADR-50 §2, owner-approved).
pub const DEFAULT_ANN_THRESHOLD: u64 = 100_000;
/// From this many vectors `AnnOptions::scale_ef` doubles the search expansion (ADR-50 §3: recall falls as the index
/// grows; doubling ef took recall@10 at 500 k from 0.969 to 0.9875 for +0.2 ms).
pub const SCALE_EF_FROM: usize = 200_000;
const META_VERSION: u32 = 1;
/// Vectors read per short read transaction while building.
const BUILD_BATCH: i64 = 8192;
/// Vectors a build inserts one by one before going parallel.
const SERIAL_SEED: usize = 4096;
/// A catch-up touching more facts than this (a re-embed of the whole memory) rebuilds instead of replaying one by one.
const REPLAY_MAX: usize = 50_000;
/// The sidecar is rebuilt (compacted) once tombstones and in-place replacements exceed this share of its live vectors…
const COMPACT_RATIO: u64 = 10;
/// …and this many.
const COMPACT_MIN: u64 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexMode {
    /// sqlite-vec only.
    Exact,
    /// The sidecar whatever the size (tests, benchmarks): built or loaded while the store opens.
    Ann,
    /// Exact below `ann_threshold` facts; from there the sidecar is built in the background and used when ready.
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnnDtype {
    /// Default: same recall as f32 at half the size (ADR-50 §3).
    F16,
    F32,
    /// Recall 0.83–0.91 (ADR-50 §6): selectable, not default.
    I8,
}

impl AnnDtype {
    fn scalar(self) -> ScalarKind {
        match self {
            Self::F16 => ScalarKind::F16,
            Self::F32 => ScalarKind::F32,
            Self::I8 => ScalarKind::I8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnOptions {
    pub dtype: AnnDtype,
    pub connectivity: usize,
    pub expansion_add: usize,
    pub expansion_search: usize,
    /// Double `expansion_search` from [`SCALE_EF_FROM`] vectors.
    pub scale_ef: bool,
    /// Re-score 4× candidates against the f32 vectors in `fact_vec` (ADR-50 §4: +1–2 recall points for ≈ 0.6 ms; off).
    pub rerank: bool,
    /// Save the sidecar after this many applied changes (also on flush, maintenance and close).
    pub save_every: u64,
    /// Threads for a build; 0 = available parallelism.
    pub build_threads: usize,
}

impl Default for AnnOptions {
    fn default() -> Self {
        Self {
            dtype: AnnDtype::F16,
            // ADR-125 §8 (sweep on 100 000 real sentences): M 32 + ef 256 gives recall@10 0.997 at 1.1 ms, and agents
            // get the same top 8 as exact search 99 % of the time; ADR-50's M 16 + ef 128 gave 0.974 and 96 %.
            connectivity: 32,
            expansion_add: 128,
            expansion_search: 256,
            scale_ef: true,
            rerank: false,
            save_every: 20_000,
            build_threads: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentOptions {
    pub index_mode: IndexMode,
    pub ann_threshold: u64,
    pub ann: AnnOptions,
}

impl Default for AgentOptions {
    fn default() -> Self {
        Self {
            index_mode: IndexMode::Auto,
            ann_threshold: DEFAULT_ANN_THRESHOLD,
            ann: AnnOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexKind {
    Exact,
    Ann,
}

/// A k-nearest-neighbour index over fact vectors: `(fact id, cosine distance)` ascending.
pub trait VectorIndex {
    fn kind(&self) -> IndexKind;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn knn(&self, query: &[f32], k: usize) -> Result<Vec<(i64, f64)>, StoreError>;
}

/// The exact index: sqlite-vec `vec0`, cosine.
pub struct Exact<'a>(pub &'a Db);

impl VectorIndex for Exact<'_> {
    fn kind(&self) -> IndexKind {
        IndexKind::Exact
    }

    fn len(&self) -> usize {
        self.0.read(vec_stats).map(|(n, _)| n as usize).unwrap_or(0)
    }

    /// Rows without a distance (a zero vector stored before `usable` existed) are skipped.
    fn knn(&self, query: &[f32], k: usize) -> Result<Vec<(i64, f64)>, StoreError> {
        self.0.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT rowid, distance FROM fact_vec WHERE embedding MATCH ?1 AND k = ?2 ORDER BY distance",
            )?;
            let rows = st.query_map(params![vec_blob(query), k as i64], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Option<f64>>(1)?))
            })?;
            let rows = rows.collect::<Result<Vec<_>, _>>()?;
            Ok(rows.into_iter().filter_map(|(id, d)| Some((id, d?))).collect())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// expansion_search 256 (ADR-125 §8), doubled from 200 000 vectors when `scale_ef` is on.
    #[test]
    fn search_expansion_doubles_from_200k_vectors() {
        let opts = AnnOptions::default();
        let s = Sidecar::new(2, &opts, SCALE_EF_FROM, 8).unwrap();
        let v = |i: usize| [(i as f32 * 0.001).cos(), (i as f32 * 0.001).sin()];
        std::thread::scope(|scope| {
            for t in 0..8 {
                let s = &s;
                scope.spawn(move || {
                    for i in (t..SCALE_EF_FROM - 1).step_by(8) {
                        s.add_new(i as i64 + 1, &v(i)).unwrap();
                    }
                });
            }
        });
        s.tune();
        assert_eq!(s.index.expansion_search(), 256);
        s.add_new(SCALE_EF_FROM as i64, &v(SCALE_EF_FROM)).unwrap();
        s.tune();
        assert_eq!(s.index.expansion_search(), 512);
        let fixed = Sidecar {
            opts: AnnOptions {
                scale_ef: false,
                ..AnnOptions::default()
            },
            ..s
        };
        fixed.tune();
        assert_eq!(fixed.index.expansion_search(), 256);
    }
}
