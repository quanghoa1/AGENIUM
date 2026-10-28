//! The usearch HNSW sidecar: built, kept in step with the journal, saved and loaded with a checksum.

use super::*;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Meta {
    pub(super) version: u32,
    pub(super) db_id: String,
    pub(super) dim: usize,
    pub(super) dtype: AnnDtype,
    pub(super) connectivity: usize,
    pub(super) applied_seq: i64,
    pub(super) count: u64,
    pub(super) sum: i64,
    pub(super) deleted: Vec<u64>,
    pub(super) replaced: u64,
    pub(super) len: u64,
    pub(super) crc32: u32,
}

/// A loaded or built HNSW index with the journal position and the `(count, sum)` of its live keys.
pub struct Sidecar {
    pub(super) index: Index,
    pub(super) opts: AnnOptions,
    pub(super) applied_seq: i64,
    pub(super) count: u64,
    pub(super) sum: i64,
    /// Keys still in the graph that are no longer live: filtered out of results, kept for navigation. usearch reuses
    /// a removed key's slot for the next add, which rewires that node's neighbours to an unrelated vector; measured
    /// here, 100 removes + 100 adds on 700 vectors left an untouched fact unable to find itself.
    pub(super) deleted: HashSet<u64>,
    /// Vectors replaced in place (remove + add: the same rewiring), counted towards compaction.
    pub(super) replaced: u64,
}

pub(super) fn ann_err(e: impl std::fmt::Display) -> StoreError {
    StoreError::Invalid(format!("ann index: {e}"))
}

impl Sidecar {
    pub(super) fn new(
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
    pub(super) fn needs_compaction(&self) -> bool {
        let churn = self.deleted.len() as u64 + self.replaced;
        churn > COMPACT_MIN && churn * COMPACT_RATIO > self.count
    }

    pub(super) fn tune(&self) {
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
    pub(super) fn set(&mut self, id: i64, vector: Option<&[f32]>) -> Result<(), StoreError> {
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
    pub(super) fn holds(&self, key: u64, v: &[f32]) -> bool {
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

    pub(super) fn add_new(&self, id: i64, v: &[f32]) -> Result<(), StoreError> {
        self.index.add(id as u64, v).map_err(ann_err)
    }

    pub(super) fn search(&self, query: &[f32], k: usize) -> Result<Vec<(i64, f64)>, StoreError> {
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
    pub(super) fn catch_up(&mut self, c: &Connection, verify: bool) -> Result<bool, StoreError> {
        c.execute_batch("BEGIN")?;
        let r = self.catch_up_in(c, verify);
        let _ = c.execute_batch("COMMIT");
        r
    }

    pub(super) fn catch_up_in(&mut self, c: &Connection, verify: bool) -> Result<bool, StoreError> {
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
    pub(super) fn build(
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

    pub(super) fn save(&self, db_path: &Path, db_id: &str) -> Result<(), StoreError> {
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
    pub(super) fn load(
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

pub(super) fn checksum(path: &Path) -> Result<(u64, u32), StoreError> {
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

pub(super) fn sync_file(path: &Path) -> Result<(), StoreError> {
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|f| f.sync_all())
        .map_err(|e| StoreError::io(path, e))
}
