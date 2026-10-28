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

/// `(count, sum(id))` of `fact_vec`, from its rowid shadow table (an index scan, not a vector scan).
fn vec_stats(c: &Connection) -> Result<(u64, i64), StoreError> {
    Ok(c.query_row(
        "SELECT count(*), coalesce(sum(rowid), 0) FROM fact_vec_rowids",
        [],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)),
    )?)
}

/// The journal's high-water mark: the last `seq` ever handed out (trimming does not lower it).
fn journal_hi(c: &Connection) -> Result<i64, StoreError> {
    Ok(c.query_row(
        "SELECT seq FROM sqlite_sequence WHERE name = 'ann_journal'",
        [],
        |r| r.get(0),
    )
    .optional()?
    .unwrap_or(0))
}

fn vector_of(c: &Connection, id: i64) -> Result<Option<Vec<f32>>, StoreError> {
    let blob: Option<Vec<u8>> = c
        .prepare_cached("SELECT embedding FROM fact_vec WHERE rowid = ?1")?
        .query_row([id], |r| r.get(0))
        .optional()?;
    Ok(blob.map(|b| decode(&b)))
}

fn decode(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>()
        .0
        .iter()
        .map(|x| f32::from_le_bytes(*x))
        .collect()
}

/// Up to `limit` fact vectors with id > `after`, ascending, read straight from vec0's shadow tables (sqlite-vec 0.1.9,
/// pinned: `<t>_rowids(rowid, id, chunk_id, chunk_offset)`, `<t>_vector_chunks00(rowid = chunk_id, vectors)`). A range
/// read: vec0 itself can only scan everything or look rows up one by one, each loading a whole 1 024-row chunk.
fn read_batch(
    c: &Connection,
    dim: usize,
    after: i64,
    limit: i64,
) -> Result<Vec<(i64, Vec<f32>)>, StoreError> {
    let rows: Vec<(i64, Option<i64>, Option<i64>)> = c
        .prepare_cached(
            "SELECT rowid, chunk_id, chunk_offset FROM fact_vec_rowids WHERE rowid > ?1 ORDER BY rowid LIMIT ?2",
        )?
        .query_map(params![after, limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let mut chunks: HashMap<i64, Vec<u8>> = HashMap::new();
    let mut st =
        c.prepare_cached("SELECT vectors FROM fact_vec_vector_chunks00 WHERE rowid = ?1")?;
    let width = dim * 4;
    let mut out = Vec::with_capacity(rows.len());
    for (id, chunk, offset) in rows {
        let (Some(chunk), Some(offset)) = (chunk, offset) else {
            return Err(StoreError::Corrupt(format!(
                "fact_vec row {id} has no chunk position"
            )));
        };
        let blob = match chunks.entry(chunk) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(st.query_row([chunk], |r| r.get(0))?)
            }
        };
        let at = offset as usize * width;
        let Some(bytes) = blob.get(at..at + width) else {
            return Err(StoreError::Corrupt(format!(
                "fact_vec row {id} is outside its chunk"
            )));
        };
        out.push((id, decode(bytes)));
    }
    Ok(out)
}

#[derive(Debug, Serialize, Deserialize)]
struct Meta {
    version: u32,
    db_id: String,
    dim: usize,
    dtype: AnnDtype,
    connectivity: usize,
    applied_seq: i64,
    count: u64,
    sum: i64,
    deleted: Vec<u64>,
    replaced: u64,
    len: u64,
    crc32: u32,
}

/// A loaded or built HNSW index with the journal position and the `(count, sum)` of its live keys.
pub struct Sidecar {
    index: Index,
    opts: AnnOptions,
    applied_seq: i64,
    count: u64,
    sum: i64,
    /// Keys still in the graph that are no longer live: filtered out of results, kept for navigation. usearch reuses
    /// a removed key's slot for the next add, which rewires that node's neighbours to an unrelated vector; measured
    /// here, 100 removes + 100 adds on 700 vectors left an untouched fact unable to find itself.
    deleted: HashSet<u64>,
    /// Vectors replaced in place (remove + add: the same rewiring), counted towards compaction.
    replaced: u64,
}

fn ann_err(e: impl std::fmt::Display) -> StoreError {
    StoreError::Invalid(format!("ann index: {e}"))
}

impl Sidecar {
    fn new(
        dim: usize,
        opts: &AnnOptions,
        capacity: usize,
        threads: usize,
    ) -> Result<Self, StoreError> {
        let index = Index::new(&IndexOptions {
            dimensions: dim,
            metric: MetricKind::Cos,
            quantization: opts.dtype.scalar(),
            connectivity: opts.connectivity,
            expansion_add: opts.expansion_add,
            expansion_search: opts.expansion_search,
            multi: false,
        })
        .map_err(ann_err)?;
        index
            .reserve_capacity_and_threads(capacity.max(1024), threads.max(1))
            .map_err(ann_err)?;
        Ok(Self {
            index,
            opts: opts.clone(),
            applied_seq: 0,
            count: 0,
            sum: 0,
            deleted: HashSet::new(),
            replaced: 0,
        })
    }

    /// Enough tombstones and replacements that a fresh build is due.
    fn needs_compaction(&self) -> bool {
        let churn = self.deleted.len() as u64 + self.replaced;
        churn > COMPACT_MIN && churn * COMPACT_RATIO > self.count
    }

    fn tune(&self) {
        let ef = if self.opts.scale_ef && self.index.size() >= SCALE_EF_FROM {
            self.opts.expansion_search * 2
        } else {
            self.opts.expansion_search
        };
        if self.index.expansion_search() != ef {
            self.index.change_expansion_search(ef);
        }
    }

    /// Makes `id` live with `vector`, or not live when `None` (a tombstone).
    fn set(&mut self, id: i64, vector: Option<&[f32]>) -> Result<(), StoreError> {
        let key = id as u64;
        let present = self.index.contains(key);
        let live = present && !self.deleted.contains(&key);
        // Already there (a build scans facts added after its start, which the catch-up then sees again): a remove
        // + add would rewire the node for nothing.
        if live && vector.is_some_and(|v| self.holds(key, v)) {
            return Ok(());
        }
        if live {
            self.count -= 1;
            self.sum = self.sum.wrapping_sub(id);
        }
        let Some(v) = vector else {
            if live {
                self.deleted.insert(key);
            }
            return Ok(());
        };
        if present {
            self.index.remove(key).map_err(ann_err)?;
            self.deleted.remove(&key);
            self.replaced += 1;
        }
        if self.index.size() >= self.index.capacity() {
            self.index
                .reserve((self.index.capacity() * 2).max(1024))
                .map_err(ann_err)?;
        }
        self.index.add(key, v).map_err(ann_err)?;
        self.count += 1;
        self.sum = self.sum.wrapping_add(id);
        Ok(())
    }

    /// Does the index hold `v`'s direction under `key` (up to its quantization)?
    fn holds(&self, key: u64, v: &[f32]) -> bool {
        let mut stored = vec![0f32; v.len()];
        if !matches!(self.index.get(key, &mut stored), Ok(n) if n > 0) {
            return false;
        }
        let (mut dot, mut a, mut b) = (0f64, 0f64, 0f64);
        for (x, y) in stored.iter().zip(v) {
            dot += *x as f64 * *y as f64;
            a += *x as f64 * *x as f64;
            b += *y as f64 * *y as f64;
        }
        let eps = match self.opts.dtype {
            AnnDtype::F32 => 1e-6,
            AnnDtype::F16 => 1e-4,
            AnnDtype::I8 => 1e-2,
        };
        a > 0.0 && b > 0.0 && 1.0 - dot / (a.sqrt() * b.sqrt()) <= eps
    }

    fn add_new(&self, id: i64, v: &[f32]) -> Result<(), StoreError> {
        self.index.add(id as u64, v).map_err(ann_err)
    }

    fn search(&self, query: &[f32], k: usize) -> Result<Vec<(i64, f64)>, StoreError> {
        let m = if self.deleted.is_empty() {
            self.index.search(query, k)
        } else {
            self.index
                .filtered_search(query, k, |key| !self.deleted.contains(&key))
        }
        .map_err(ann_err)?;
        Ok(m.keys
            .into_iter()
            .zip(m.distances)
            .map(|(key, d)| (key as i64, d as f64))
            .collect())
    }

    /// Applies the journal past `applied_seq` and checks `(count, sum)` against `fact_vec`, in one read snapshot.
    /// `Ok(false)`: too many changes to replay one by one (the caller rebuilds).
    fn catch_up(&mut self, c: &Connection, verify: bool) -> Result<bool, StoreError> {
        c.execute_batch("BEGIN")?;
        let r = self.catch_up_in(c, verify);
        let _ = c.execute_batch("COMMIT");
        r
    }

    fn catch_up_in(&mut self, c: &Connection, verify: bool) -> Result<bool, StoreError> {
        let hi = journal_hi(c)?;
        if self.applied_seq > hi {
            return Err(StoreError::Corrupt(format!(
                "sidecar covers journal entry {} but the journal ends at {hi}",
                self.applied_seq
            )));
        }
        if self.applied_seq < hi {
            let (n, lo): (i64, Option<i64>) = c.query_row(
                "SELECT count(*), min(seq) FROM ann_journal WHERE seq > ?1",
                [self.applied_seq],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if n != hi - self.applied_seq || lo != Some(self.applied_seq + 1) {
                return Err(StoreError::Corrupt(format!(
                    "journal no longer holds entries {}..={hi}",
                    self.applied_seq + 1
                )));
            }
            let touched: BTreeSet<i64> = c
                .prepare_cached("SELECT fact_id FROM ann_journal WHERE seq > ?1")?
                .query_map([self.applied_seq], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            if touched.len() > REPLAY_MAX {
                return Ok(false);
            }
            for id in touched {
                let v = vector_of(c, id)?;
                self.set(id, v.as_deref())?;
            }
            self.applied_seq = hi;
            self.tune();
        }
        if verify {
            let (n, sum) = vec_stats(c)?;
            if (n, sum) != (self.count, self.sum)
                || self.index.size() as u64 != self.count + self.deleted.len() as u64
            {
                return Err(StoreError::Corrupt(format!(
                    "sidecar holds {} vectors (id sum {}), fact_vec {n} (id sum {sum})",
                    self.count, self.sum
                )));
            }
        }
        Ok(true)
    }

    /// Builds from `fact_vec` in short read transactions. Changes made meanwhile are journaled after the position
    /// taken first, and the final catch-up resets every fact they touched.
    /// `floor` keeps the journal from being trimmed past the build's starting position while it runs.
    fn build(
        db: &Db,
        dim: usize,
        opts: &AnnOptions,
        stop: &AtomicBool,
        floor: &AtomicI64,
    ) -> Result<Option<Sidecar>, StoreError> {
        floor.store(0, Ordering::SeqCst);
        let (start, (n, _)) = db.read(|c| Ok::<_, StoreError>((journal_hi(c)?, vec_stats(c)?)))?;
        floor.store(start, Ordering::SeqCst);
        let threads = match opts.build_threads {
            0 => std::thread::available_parallelism().map_or(4, |n| n.get()),
            n => n,
        };
        let mut s = Sidecar::new(dim, opts, n as usize + n as usize / 8, threads)?;
        s.applied_seq = start;
        let mut after = 0;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(None);
            }
            let batch = db.read(|c| read_batch(c, dim, after, BUILD_BATCH))?;
            let Some(last) = batch.last() else { break };
            after = last.0;
            if s.index.size() + batch.len() > s.index.capacity() {
                s.index
                    .reserve_capacity_and_threads(
                        (s.index.capacity() * 2).max(s.index.size() + batch.len()),
                        threads,
                    )
                    .map_err(ann_err)?;
            }
            // The first vectors go in one by one: concurrent inserts into a near-empty graph connect it poorly
            // (measured: 8 of 600 facts could not find themselves after a 4-thread build, 0 after a serial one).
            let serial = SERIAL_SEED.saturating_sub(s.index.size()).min(batch.len());
            for (id, v) in &batch[..serial] {
                s.add_new(*id, v)?;
            }
            let rest = &batch[serial..];
            let per = rest.len().div_ceil(threads).max(1);
            let s_ref = &s;
            let results: Vec<Result<(), StoreError>> = std::thread::scope(|scope| {
                let hs: Vec<_> = rest
                    .chunks(per)
                    .map(|part| {
                        scope.spawn(move || {
                            part.iter().try_for_each(|(id, v)| s_ref.add_new(*id, v))
                        })
                    })
                    .collect();
                hs.into_iter()
                    .map(|h| {
                        h.join()
                            .unwrap_or_else(|_| Err(ann_err("build thread panicked")))
                    })
                    .collect()
            });
            results.into_iter().collect::<Result<(), _>>()?;
            for (id, _) in &batch {
                s.count += 1;
                s.sum = s.sum.wrapping_add(*id);
            }
        }
        s.tune();
        Ok(Some(s))
    }

    fn save(&self, db_path: &Path, db_id: &str) -> Result<(), StoreError> {
        let file = sidecar_path(db_path, ".ann");
        let tmp = sidecar_path(db_path, ".ann.tmp");
        let meta_file = sidecar_path(db_path, ".ann.json");
        let meta_tmp = sidecar_path(db_path, ".ann.json.tmp");
        let tmp_str = tmp
            .to_str()
            .ok_or_else(|| StoreError::Invalid(format!("{}: path is not UTF-8", tmp.display())))?;
        self.index.save(tmp_str).map_err(ann_err)?;
        let (len, crc32) = checksum(&tmp)?;
        sync_file(&tmp)?;
        let meta = Meta {
            version: META_VERSION,
            db_id: db_id.to_owned(),
            dim: self.index.dimensions(),
            dtype: self.opts.dtype,
            connectivity: self.opts.connectivity,
            applied_seq: self.applied_seq,
            count: self.count,
            sum: self.sum,
            deleted: self.deleted.iter().copied().collect(),
            replaced: self.replaced,
            len,
            crc32,
        };
        {
            let mut f = fs::File::create(&meta_tmp).map_err(|e| StoreError::io(&meta_tmp, e))?;
            f.write_all(&serde_json::to_vec_pretty(&meta)?)
                .and_then(|_| f.sync_all())
                .map_err(|e| StoreError::io(&meta_tmp, e))?;
        }
        // The index first: a crash between the renames leaves a new index with the old meta, which fails the
        // length/crc check and is rebuilt.
        fs::rename(&tmp, &file).map_err(|e| StoreError::io(&file, e))?;
        fs::rename(&meta_tmp, &meta_file).map_err(|e| StoreError::io(&meta_file, e))?;
        Ok(())
    }

    /// Loads the saved sidecar if it is intact and matches this database and options (not yet caught up).
    fn load(
        db_path: &Path,
        dim: usize,
        opts: &AnnOptions,
        db_id: &str,
    ) -> Result<Sidecar, StoreError> {
        let meta_file = sidecar_path(db_path, ".ann.json");
        let file = sidecar_path(db_path, ".ann");
        let raw = fs::read(&meta_file).map_err(|e| StoreError::io(&meta_file, e))?;
        let meta: Meta = serde_json::from_slice(&raw)?;
        if meta.version != META_VERSION
            || meta.db_id != db_id
            || meta.dim != dim
            || meta.dtype != opts.dtype
            || meta.connectivity != opts.connectivity
        {
            return Err(StoreError::Invalid(
                "sidecar was built for another database or other options".into(),
            ));
        }
        if checksum(&file)? != (meta.len, meta.crc32) {
            return Err(StoreError::Corrupt(
                "sidecar length or crc32 mismatch".into(),
            ));
        }
        let index = Index::new(&IndexOptions {
            dimensions: dim,
            metric: MetricKind::Cos,
            quantization: opts.dtype.scalar(),
            connectivity: opts.connectivity,
            expansion_add: opts.expansion_add,
            expansion_search: opts.expansion_search,
            multi: false,
        })
        .map_err(ann_err)?;
        let path = file
            .to_str()
            .ok_or_else(|| StoreError::Invalid(format!("{}: path is not UTF-8", file.display())))?;
        index.load(path).map_err(ann_err)?;
        index.change_expansion_add(opts.expansion_add);
        index.change_expansion_search(opts.expansion_search);
        let deleted: HashSet<u64> = meta.deleted.into_iter().collect();
        if index.size() as u64 != meta.count + deleted.len() as u64
            || index.dimensions() != dim
            || deleted.iter().any(|k| !index.contains(*k))
        {
            return Err(StoreError::Corrupt(
                "sidecar content does not match its metadata".into(),
            ));
        }
        let s = Sidecar {
            index,
            opts: opts.clone(),
            applied_seq: meta.applied_seq,
            count: meta.count,
            sum: meta.sum,
            deleted,
            replaced: meta.replaced,
        };
        s.tune();
        Ok(s)
    }
}

impl VectorIndex for Sidecar {
    fn kind(&self) -> IndexKind {
        IndexKind::Ann
    }

    fn len(&self) -> usize {
        self.count as usize
    }

    fn knn(&self, query: &[f32], k: usize) -> Result<Vec<(i64, f64)>, StoreError> {
        self.search(query, k)
    }
}

pub(crate) fn sidecar_path(db_path: &Path, suffix: &str) -> PathBuf {
    let mut p = db_path.as_os_str().to_owned();
    p.push(suffix);
    PathBuf::from(p)
}

/// The sidecar files of an agent database (deleting an agent removes them with it).
pub fn sidecar_files(db_path: &Path) -> Vec<PathBuf> {
    [".ann", ".ann.json", ".ann.tmp", ".ann.json.tmp"]
        .iter()
        .map(|s| sidecar_path(db_path, s))
        .collect()
}

fn checksum(path: &Path) -> Result<(u64, u32), StoreError> {
    let mut f = fs::File::open(path).map_err(|e| StoreError::io(path, e))?;
    let mut h = crc32fast::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut len = 0u64;
    loop {
        let n = f.read(&mut buf).map_err(|e| StoreError::io(path, e))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        len += n as u64;
    }
    Ok((len, h.finalize()))
}

fn sync_file(path: &Path) -> Result<(), StoreError> {
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| StoreError::io(path, e))
}

/// What the sidecar is doing, for status and tests.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AnnStatus {
    pub mode: IndexMode,
    /// The index searches use now.
    pub active: IndexKind,
    pub building: bool,
    /// Vectors in the sidecar (0 when none is loaded).
    pub size: usize,
    /// Why the last saved sidecar was not used (missing, corrupt, behind the journal, ...), if it was not.
    pub last_discard: Option<String>,
    /// Builds completed since the store opened.
    pub builds: u64,
}

/// The vector-index state of one agent database: the active sidecar (if any) and its background build.
pub(crate) struct Vectors {
    db: Arc<Db>,
    dim: usize,
    opts: AgentOptions,
    ann: RwLock<Option<Sidecar>>,
    building: Mutex<Option<JoinHandle<()>>>,
    build_done: Condvar,
    stop: AtomicBool,
    /// Applied changes since the last save.
    unsaved: AtomicU64,
    /// `(vector count, journal position)` last counted: the count can have grown by at most the journal entries
    /// since, so Auto mode counts again only when that bound reaches the threshold.
    counted: Mutex<(u64, i64)>,
    builds: AtomicU64,
    last_discard: Mutex<Option<String>>,
    db_id: OnceLock<String>,
    /// The journal is never trimmed past this position (a running build's start); `i64::MAX` when no build runs.
    floor: AtomicI64,
    /// One save at a time: a compaction build saves its sidecar while writes may save the active one, through the same
    /// temporary files.
    saving: Mutex<()>,
}

impl Vectors {
    pub(crate) fn open(
        db: Arc<Db>,
        dim: usize,
        opts: AgentOptions,
    ) -> Result<Arc<Vectors>, StoreError> {
        let v = Arc::new(Vectors {
            db,
            dim,
            opts,
            ann: RwLock::new(None),
            building: Mutex::new(None),
            build_done: Condvar::new(),
            stop: AtomicBool::new(false),
            unsaved: AtomicU64::new(0),
            counted: Mutex::new((0, 0)),
            builds: AtomicU64::new(0),
            last_discard: Mutex::new(None),
            db_id: OnceLock::new(),
            floor: AtomicI64::new(i64::MAX),
            saving: Mutex::new(()),
        });
        match v.opts.index_mode {
            IndexMode::Exact => {}
            // Forced: ready when `open` returns.
            IndexMode::Ann => {
                if let Some(s) = v.load_or_build(false)? {
                    v.install(s);
                }
            }
            IndexMode::Auto => {
                if v.over_threshold()? {
                    v.start_build(false);
                }
            }
        }
        Ok(v)
    }

    fn read_ann(&self) -> RwLockReadGuard<'_, Option<Sidecar>> {
        self.ann.read().unwrap_or_else(|p| p.into_inner())
    }

    fn write_ann(&self) -> RwLockWriteGuard<'_, Option<Sidecar>> {
        self.ann.write().unwrap_or_else(|p| p.into_inner())
    }

    pub(crate) fn options(&self) -> &AgentOptions {
        &self.opts
    }

    fn over_threshold(&self) -> Result<bool, StoreError> {
        let (n, hi) = self
            .db
            .read(|c| Ok::<_, StoreError>((vec_stats(c)?.0, journal_hi(c)?)))?;
        *self.counted.lock().unwrap_or_else(|p| p.into_inner()) = (n, hi);
        Ok(n >= self.opts.ann_threshold)
    }

    /// [`Self::over_threshold`], counting only when the journal says the threshold may have been reached.
    fn maybe_over_threshold(&self) -> Result<bool, StoreError> {
        let hi = self.db.read(journal_hi)?;
        let (n, at) = *self.counted.lock().unwrap_or_else(|p| p.into_inner());
        if n + (hi - at).max(0) as u64 >= self.opts.ann_threshold {
            self.over_threshold()
        } else {
            Ok(false)
        }
    }

    /// The identity a sidecar must carry to be loaded for this database (created on first use).
    fn db_id(&self) -> Result<String, StoreError> {
        if let Some(id) = self.db_id.get() {
            return Ok(id.clone());
        }
        let id = self.db.write(|tx| {
            let id: Option<String> = tx
                .query_row(
                    "SELECT value FROM agent_meta WHERE key = 'ann_db_id'",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = id {
                return Ok(id);
            }
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos());
            let seed = format!(
                "{}:{nanos}:{}",
                self.db.path().display(),
                std::process::id()
            );
            use sha2::Digest;
            let id = crate::backup::hex(&sha2::Sha256::digest(seed.as_bytes()))[..32].to_owned();
            tx.execute(
                "INSERT INTO agent_meta(key, value) VALUES ('ann_db_id', ?1)",
                [&id],
            )?;
            Ok::<_, StoreError>(id)
        })?;
        Ok(self.db_id.get_or_init(|| id).clone())
    }

    fn discard(&self, why: String) {
        *self.last_discard.lock().unwrap_or_else(|p| p.into_inner()) = Some(why);
    }

    /// The saved sidecar if it is valid, caught up and not due for compaction, else a fresh build (`fresh` skips the
    /// saved one). `None` when stopped.
    fn load_or_build(&self, fresh: bool) -> Result<Option<Sidecar>, StoreError> {
        let db_id = self.db_id()?;
        if !fresh && sidecar_path(self.db.path(), ".ann.json").exists() {
            let loaded = Sidecar::load(self.db.path(), self.dim, &self.opts.ann, &db_id).and_then(
                |mut s| match self.db.read(|c| s.catch_up(c, true))? {
                    true => Ok(s),
                    false => Err(StoreError::Invalid(
                        "too many changes since the sidecar was saved".into(),
                    )),
                },
            );
            match loaded {
                Ok(s) if !s.needs_compaction() => return Ok(Some(s)),
                Ok(_) => {}
                Err(e) => self.discard(e.to_string()),
            }
        }
        let built = Sidecar::build(&self.db, self.dim, &self.opts.ann, &self.stop, &self.floor);
        let r = (|| {
            let Some(mut s) = built? else { return Ok(None) };
            self.db.read(|c| s.catch_up(c, true))?;
            self.builds.fetch_add(1, Ordering::Relaxed);
            let _one = self.saving.lock().unwrap_or_else(|p| p.into_inner());
            s.save(self.db.path(), &db_id)?;
            Ok(Some(s))
        })();
        self.floor.store(i64::MAX, Ordering::SeqCst);
        if let Ok(Some(s)) = &r {
            self.trim(s.applied_seq)?;
        }
        r
    }

    /// Makes `s` the active index (replacing a compacted one): the last journal entries are applied under the write
    /// lock, so no write committed before the switch is missed (later ones apply themselves through `after_write`).
    fn install(&self, mut s: Sidecar) {
        let mut slot = self.write_ann();
        let before = s.applied_seq;
        match self.db.read(|c| s.catch_up(c, true)) {
            Ok(true) => {
                // Changes applied here, or a replaced sidecar that may have saved over this one's files, are
                // unsaved: the next save (maintenance, close) writes this sidecar.
                if s.applied_seq != before || slot.is_some() {
                    self.unsaved.fetch_add(1, Ordering::Relaxed);
                }
                *slot = Some(s);
            }
            Ok(false) => self.discard("too many changes during the build".into()),
            Err(e) => self.discard(e.to_string()),
        }
    }

    /// Builds (or loads) in the background; exact search, or the sidecar being compacted, serves meanwhile.
    fn start_build(self: &Arc<Self>, fresh: bool) {
        let mut b = self.building.lock().unwrap_or_else(|p| p.into_inner());
        if b.as_ref().is_some_and(|h| !h.is_finished()) || self.stop.load(Ordering::Relaxed) {
            return;
        }
        let me = Arc::clone(self);
        *b = Some(std::thread::spawn(move || {
            match me.load_or_build(fresh) {
                Ok(Some(s)) => me.install(s),
                Ok(None) => {}
                Err(e) => me.discard(format!("build failed: {e}")),
            }
            let _g = me.building.lock().unwrap_or_else(|p| p.into_inner());
            me.build_done.notify_all();
        }));
    }

    fn is_building(&self) -> bool {
        self.building
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .is_some_and(|h| !h.is_finished())
    }

    /// Waits until no build runs (tests, benchmarks, the CLI). `false` on timeout.
    pub(crate) fn wait_ready(&self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        let mut b = self.building.lock().unwrap_or_else(|p| p.into_inner());
        while b.as_ref().is_some_and(|h| !h.is_finished()) {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return false;
            }
            b = self
                .build_done
                .wait_timeout(b, left.min(Duration::from_millis(50)))
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        true
    }

    /// KNN through the sidecar when it is active; `None` means "use exact". A search error discards the sidecar and
    /// schedules a rebuild.
    pub(crate) fn knn(self: &Arc<Self>, query: &[f32], k: usize) -> Option<Vec<(i64, f64)>> {
        let r = {
            let slot = self.read_ann();
            slot.as_ref()?.search(query, k)
        };
        match r {
            Ok(hits) => Some(hits),
            Err(e) => {
                self.drop_ann(format!("search failed: {e}"));
                None
            }
        }
    }

    fn drop_ann(self: &Arc<Self>, why: String) {
        *self.write_ann() = None;
        self.discard(why);
        if self.opts.index_mode != IndexMode::Exact {
            self.start_build(true);
        }
    }

    /// Called after every committed write that touched `fact_vec`: the active sidecar applies the new journal entries
    /// before the write returns (read-your-writes); in Auto mode a crossed threshold starts the build.
    pub(crate) fn after_write(self: &Arc<Self>) {
        let applied = {
            let mut slot = self.write_ann();
            match slot.as_mut() {
                None => None,
                Some(s) => {
                    let before = s.applied_seq;
                    Some(
                        self.db
                            .read(|c| s.catch_up(c, false))
                            .map(|ok| (ok, s.applied_seq - before, s.needs_compaction())),
                    )
                }
            }
        };
        match applied {
            Some(Ok((true, delta, compact))) => {
                let unsaved =
                    self.unsaved.fetch_add(delta as u64, Ordering::Relaxed) + delta as u64;
                if unsaved >= self.opts.ann.save_every {
                    let _ = self.save();
                }
                if compact && !self.is_building() {
                    self.start_build(true);
                }
            }
            Some(Ok((false, _, _))) => self.drop_ann("too many changes to replay".into()),
            Some(Err(e)) => self.drop_ann(format!("catch-up failed: {e}")),
            None => {
                if self.opts.index_mode == IndexMode::Auto
                    && !self.is_building()
                    && self.maybe_over_threshold().unwrap_or(false)
                {
                    self.start_build(false);
                }
            }
        }
    }

    /// Saves the active sidecar (under a read lock: searches go on) and trims the journal it covers.
    pub(crate) fn save(&self) -> Result<(), StoreError> {
        let db_id = self.db_id()?;
        let seq = {
            let slot = self.read_ann();
            let Some(s) = slot.as_ref() else {
                return Ok(());
            };
            let _one = self.saving.lock().unwrap_or_else(|p| p.into_inner());
            s.save(self.db.path(), &db_id)?;
            s.applied_seq
        };
        self.unsaved.store(0, Ordering::Relaxed);
        self.trim(seq)
    }

    /// Drops journal entries up to `upto`, but never past a running build's start or what the active sidecar has
    /// applied (a fresh build saves before it replaces the active one, whose catch-up still needs the entries).
    fn trim(&self, upto: i64) -> Result<(), StoreError> {
        let mut upto = upto.min(self.floor.load(Ordering::SeqCst));
        if let Some(s) = self.read_ann().as_ref() {
            upto = upto.min(s.applied_seq);
        }
        self.db.write(|tx| {
            Ok::<_, StoreError>(tx.execute("DELETE FROM ann_journal WHERE seq <= ?1", [upto])?)
        })?;
        Ok(())
    }

    /// Maintenance tick: saves unsaved sidecar changes; with no sidecar at all (exact mode, below the threshold) the
    /// journal is not needed and is emptied, so it does not grow for the life of an agent.
    pub(crate) fn maintain(&self) -> Result<(), StoreError> {
        let active = self.read_ann().is_some();
        if active {
            if self.unsaved.load(Ordering::Relaxed) > 0 {
                self.save()?;
            }
        } else {
            // Under the build lock: a build starting meanwhile needs the entries past the position it takes first.
            let b = self.building.lock().unwrap_or_else(|p| p.into_inner());
            if b.as_ref().is_none_or(|h| h.is_finished()) {
                let meta = sidecar_path(self.db.path(), ".ann.json");
                // Auto mode back below the threshold: a sidecar from when the memory was larger would keep the
                // journal growing; it is derived data, so it goes (a later build starts from `fact_vec`).
                if meta.exists()
                    && self.opts.index_mode == IndexMode::Auto
                    && !self.over_threshold()?
                {
                    for f in sidecar_files(self.db.path()) {
                        let _ = fs::remove_file(f);
                    }
                }
                if !meta.exists() {
                    let hi = self.db.read(journal_hi)?;
                    self.trim(hi)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn status(&self) -> AnnStatus {
        let slot = self.read_ann();
        AnnStatus {
            mode: self.opts.index_mode,
            active: if slot.is_some() {
                IndexKind::Ann
            } else {
                IndexKind::Exact
            },
            building: self.is_building(),
            size: slot.as_ref().map_or(0, |s| s.count as usize),
            last_discard: self
                .last_discard
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
            builds: self.builds.load(Ordering::Relaxed),
        }
    }

    /// Stops a running build and saves the active sidecar (on close).
    pub(crate) fn close(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let h = self
            .building
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(h) = h {
            let _ = h.join();
        }
        if self.unsaved.load(Ordering::Relaxed) > 0 {
            let _ = self.save();
        }
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
