//! An agent store's vectors: exact below the threshold, the sidecar from there, built in the background, its status.

use super::*;

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
    pub(super) db: Arc<Db>,
    pub(super) dim: usize,
    pub(super) opts: AgentOptions,
    pub(super) ann: RwLock<Option<Sidecar>>,
    pub(super) building: Mutex<Option<JoinHandle<()>>>,
    pub(super) build_done: Condvar,
    pub(super) stop: AtomicBool,
    /// Applied changes since the last save.
    pub(super) unsaved: AtomicU64,
    /// `(vector count, journal position)` last counted: the count can have grown by at most the journal entries
    /// since, so Auto mode counts again only when that bound reaches the threshold.
    pub(super) counted: Mutex<(u64, i64)>,
    pub(super) builds: AtomicU64,
    pub(super) last_discard: Mutex<Option<String>>,
    pub(super) db_id: OnceLock<String>,
    /// The journal is never trimmed past this position (a running build's start); `i64::MAX` when no build runs.
    pub(super) floor: AtomicI64,
    /// One save at a time: a compaction build saves its sidecar while writes may save the active one, through the same
    /// temporary files.
    pub(super) saving: Mutex<()>,
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

    pub(super) fn read_ann(&self) -> RwLockReadGuard<'_, Option<Sidecar>> {
        self.ann.read().unwrap_or_else(|p| p.into_inner())
    }

    pub(super) fn write_ann(&self) -> RwLockWriteGuard<'_, Option<Sidecar>> {
        self.ann.write().unwrap_or_else(|p| p.into_inner())
    }

    pub(crate) fn options(&self) -> &AgentOptions {
        &self.opts
    }

    pub(super) fn over_threshold(&self) -> Result<bool, StoreError> {
        let (n, hi) = self
            .db
            .read(|c| Ok::<_, StoreError>((vec_stats(c)?.0, journal_hi(c)?)))?;
        *self.counted.lock().unwrap_or_else(|p| p.into_inner()) = (n, hi);
        Ok(n >= self.opts.ann_threshold)
    }

    /// [`Self::over_threshold`], counting only when the journal says the threshold may have been reached.
    pub(super) fn maybe_over_threshold(&self) -> Result<bool, StoreError> {
        let hi = self.db.read(journal_hi)?;
        let (n, at) = *self.counted.lock().unwrap_or_else(|p| p.into_inner());
        if n + (hi - at).max(0) as u64 >= self.opts.ann_threshold {
            self.over_threshold()
        } else {
            Ok(false)
        }
    }

    /// The identity a sidecar must carry to be loaded for this database (created on first use).
    pub(super) fn db_id(&self) -> Result<String, StoreError> {
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

    pub(super) fn discard(&self, why: String) {
        *self.last_discard.lock().unwrap_or_else(|p| p.into_inner()) = Some(why);
    }

    /// The saved sidecar if it is valid, caught up and not due for compaction, else a fresh build (`fresh` skips the
    /// saved one). `None` when stopped.
    pub(super) fn load_or_build(&self, fresh: bool) -> Result<Option<Sidecar>, StoreError> {
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
    pub(super) fn install(&self, mut s: Sidecar) {
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
    pub(super) fn start_build(self: &Arc<Self>, fresh: bool) {
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

    pub(super) fn is_building(&self) -> bool {
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

    pub(super) fn drop_ann(self: &Arc<Self>, why: String) {
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
    pub(super) fn trim(&self, upto: i64) -> Result<(), StoreError> {
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
