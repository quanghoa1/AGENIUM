//! One SQLite database: a serialized writer connection, a fair pool of read connections (WAL snapshots) and a
//! process lock file. Concurrency model from ADR-50 §8: `BEGIN IMMEDIATE` writes, condvar-queued readers with a
//! large mmap, and a WAL gate that pauses reads for a TRUNCATE checkpoint when the WAL grows past a limit.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, Once};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};

use crate::StoreError;

#[derive(Debug, Clone)]
pub struct DbOptions {
    /// `synchronous=FULL` (core: the event log is the source of truth) or NORMAL (agent memory).
    pub synchronous_full: bool,
    pub readers: usize,
    pub mmap_bytes: i64,
    pub busy_timeout: Duration,
    /// How long to wait for another process to release the database lock file.
    pub lock_wait: Duration,
    /// WAL size above which [`Db::maintain`] runs an exclusive TRUNCATE checkpoint.
    pub wal_limit_bytes: u64,
}

impl Default for DbOptions {
    fn default() -> Self {
        Self {
            synchronous_full: true,
            readers: 4,
            mmap_bytes: 1 << 30,
            busy_timeout: Duration::from_secs(5),
            lock_wait: Duration::from_millis(1500),
            wal_limit_bytes: 64 << 20,
        }
    }
}

static REGISTER_VEC: Once = Once::new();

/// Registers sqlite-vec for every connection opened afterwards in this process.
pub fn register_sqlite_vec() {
    REGISTER_VEC.call_once(|| {
        // SAFETY: `sqlite3_vec_init` has the signature SQLite expects for an auto extension entry point; the
        // transmute only erases the argument types, exactly as documented by sqlite-vec for static linking.
        unsafe {
            type Init = unsafe extern "C" fn(
                *mut rusqlite::ffi::sqlite3,
                *mut *mut std::os::raw::c_char,
                *const rusqlite::ffi::sqlite3_api_routines,
            ) -> std::os::raw::c_int;
            let f: Init =
                std::mem::transmute::<*const (), Init>(sqlite_vec::sqlite3_vec_init as *const ());
            rusqlite::ffi::sqlite3_auto_extension(Some(f));
        }
    });
}

struct PoolState {
    free: Vec<Connection>,
    gate: bool,
}

struct ReaderPool {
    state: Mutex<PoolState>,
    cv: Condvar,
    size: usize,
}

impl ReaderPool {
    fn lock(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn acquire(&self) -> Connection {
        let mut st = self.lock();
        loop {
            if !st.gate {
                if let Some(c) = st.free.pop() {
                    return c;
                }
            }
            st = self.cv.wait(st).unwrap_or_else(|p| p.into_inner());
        }
    }

    fn release(&self, c: Connection) {
        self.lock().free.push(c);
        self.cv.notify_all();
    }
}

/// A pooled read connection, handed back when dropped — also when the closure using it panics. Before, a panic lost
/// the connection for good: the pool shrank and the exclusive checkpoint, which waits for every reader, never ran.
struct Lease<'a> {
    pool: &'a ReaderPool,
    conn: Option<Connection>,
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Some(c) = self.conn.take() {
            // A read transaction left open (a panic between BEGIN and COMMIT) would pin an old snapshot.
            if !c.is_autocommit() {
                let _ = c.execute_batch("ROLLBACK");
            }
            self.pool.release(c);
        }
    }
}

pub struct Db {
    path: PathBuf,
    writer: Mutex<Connection>,
    readers: ReaderPool,
    opts: DbOptions,
    /// Committed writes, for the write-path WAL gate.
    writes: AtomicU64,
    _lock: File,
}

/// Every this many committed writes, `write` checks the WAL size and runs the WAL gate past the limit.
const WAL_CHECK_EVERY: u64 = 32;

/// How long the write-path WAL gate waits for in-flight reads before giving up for now (it retries later).
const WAL_GATE_WAIT: Duration = Duration::from_secs(2);

/// How long the exclusive checkpoint (maintenance tick, [`Db::checkpoint_exclusive`]) waits for in-flight reads.
pub const EXCLUSIVE_WAIT: Duration = Duration::from_secs(30);

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db").field("path", &self.path).finish()
    }
}

fn lock_file(path: &Path, wait: Duration) -> Result<File, StoreError> {
    let lock_path = path.with_extension(format!(
        "{}.lock",
        path.extension().and_then(|e| e.to_str()).unwrap_or("db")
    ));
    let f = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| StoreError::io(&lock_path, e))?;
    let start = Instant::now();
    loop {
        match f.try_lock() {
            Ok(()) => return Ok(f),
            Err(TryLockError::WouldBlock) if start.elapsed() < wait => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(TryLockError::WouldBlock) => return Err(StoreError::Locked(path.to_path_buf())),
            Err(TryLockError::Error(e)) => return Err(StoreError::io(&lock_path, e)),
        }
    }
}

fn configure(c: &Connection, opts: &DbOptions, reader: bool) -> Result<(), StoreError> {
    c.busy_timeout(opts.busy_timeout)?;
    c.pragma_update(None, "foreign_keys", "ON")?;
    c.pragma_update(
        None,
        "synchronous",
        if opts.synchronous_full {
            "FULL"
        } else {
            "NORMAL"
        },
    )?;
    c.pragma_update(None, "temp_store", "MEMORY")?;
    if reader {
        c.pragma_update(None, "mmap_size", opts.mmap_bytes)?;
        c.pragma_update(None, "query_only", "ON")?;
    } else {
        c.pragma_update(None, "journal_size_limit", 64i64 << 20)?;
    }
    Ok(())
}

impl Db {
    /// Opens (creating if needed) the database at `path`. `init` runs on the writer connection before the
    /// readers are opened (migrations); `per_conn` runs on every connection (e.g. checks).
    pub fn open(
        path: &Path,
        opts: DbOptions,
        init: impl FnOnce(&mut Connection) -> Result<(), StoreError>,
    ) -> Result<Db, StoreError> {
        register_sqlite_vec();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| StoreError::io(dir, e))?;
        }
        let lock = lock_file(path, opts.lock_wait)?;
        let mut writer = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let mode: String = writer.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(StoreError::Invalid(format!(
                "{}: cannot enable WAL (got {mode})",
                path.display()
            )));
        }
        configure(&writer, &opts, false)?;
        init(&mut writer)?;
        let mut free = Vec::with_capacity(opts.readers);
        for _ in 0..opts.readers.max(1) {
            let r = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            configure(&r, &opts, true)?;
            free.push(r);
        }
        let size = free.len();
        Ok(Db {
            path: path.to_path_buf(),
            writer: Mutex::new(writer),
            readers: ReaderPool {
                state: Mutex::new(PoolState { free, gate: false }),
                cv: Condvar::new(),
                size,
            },
            opts,
            writes: AtomicU64::new(0),
            _lock: lock,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Runs `f` on a pooled read connection (a consistent WAL snapshot for the duration of `f` when `f` opens a
    /// read transaction; single statements are always consistent).
    pub fn read<T, E: From<StoreError>>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, E>,
    ) -> Result<T, E> {
        let lease = Lease {
            pool: &self.readers,
            conn: Some(self.readers.acquire()),
        };
        f(lease.conn.as_ref().expect("leased connection"))
    }

    /// [`Db::read`] with the connection mutable, for what changes the connection rather than the data (a progress
    /// handler; rusqlite 0.41 takes `&mut`). The caller restores what it set before returning.
    pub fn read_mut<T, E: From<StoreError>>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, E>,
    ) -> Result<T, E> {
        let mut lease = Lease {
            pool: &self.readers,
            conn: Some(self.readers.acquire()),
        };
        f(lease.conn.as_mut().expect("leased connection"))
    }

    /// Runs `f` inside `BEGIN IMMEDIATE` on the writer; commits on `Ok`, rolls back on `Err`.
    pub fn write<T, E: From<StoreError>>(
        &self,
        f: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
    ) -> Result<T, E> {
        let v = {
            let mut w = self.writer.lock().unwrap_or_else(|p| p.into_inner());
            let tx = w
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|e| E::from(StoreError::from(e)))?;
            let v = f(&tx)?;
            tx.commit().map_err(|e| E::from(StoreError::from(e)))?;
            v
        };
        // The WAL gate in the write path (STORE-SCALE §4, re-measured 2026-10-25): with busy readers a passive
        // checkpoint never restarts the WAL, and agent databases had no maintenance tick at all, so reads slowed
        // without bound under writes. Checked every few writes, after the writer is released; it waits for in-flight
        // reads only briefly and otherwise retries on a later write.
        if self.writes.fetch_add(1, Ordering::Relaxed) % WAL_CHECK_EVERY == WAL_CHECK_EVERY - 1
            && self.wal_bytes() > self.opts.wal_limit_bytes
        {
            // A failure leaves the WAL as it is; the next check retries.
            let _ = self.checkpoint_gate(WAL_GATE_WAIT);
        }
        Ok(v)
    }

    /// Exclusive access to the writer connection (backups, maintenance, schema work).
    pub fn with_writer<T>(&self, f: impl FnOnce(&mut Connection) -> T) -> T {
        let mut w = self.writer.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut w)
    }

    fn wal_path(&self) -> PathBuf {
        let mut p = self.path.clone().into_os_string();
        p.push("-wal");
        PathBuf::from(p)
    }

    pub fn wal_bytes(&self) -> u64 {
        std::fs::metadata(self.wal_path())
            .map(|m| m.len())
            .unwrap_or(0)
    }

    /// Maintenance tick: passive checkpoint; if the WAL is still above the limit, pause new reads, drain the
    /// in-flight ones and TRUNCATE (the WAL gate). Returns the WAL size afterwards: when reads did not drain within
    /// [`EXCLUSIVE_WAIT`] the WAL stays as it is and the next tick retries (the tick's backups still run).
    pub fn maintain(&self) -> Result<u64, StoreError> {
        self.with_writer(|w| w.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(())))?;
        if self.wal_bytes() > self.opts.wal_limit_bytes {
            self.checkpoint_gate(EXCLUSIVE_WAIT)?;
        }
        Ok(self.wal_bytes())
    }

    /// TRUNCATE checkpoint with all readers parked. Waits at most [`EXCLUSIVE_WAIT`] for in-flight reads, then fails
    /// with the pool reopened: it waited forever, so one read that never ended (or a reader lost to a panic) hung
    /// every later read behind the closed gate.
    pub fn checkpoint_exclusive(&self) -> Result<(), StoreError> {
        if self.checkpoint_gate(EXCLUSIVE_WAIT)? {
            Ok(())
        } else {
            Err(StoreError::Conflict(format!(
                "{}: reads still in progress after {}s; checkpoint not run",
                self.path.display(),
                EXCLUSIVE_WAIT.as_secs()
            )))
        }
    }

    /// The WAL gate: closes the reader pool to new reads, waits up to `wait` for in-flight reads to finish, then runs
    /// a TRUNCATE checkpoint and reopens the pool. Returns `false` (nothing done, pool reopened) when reads did not
    /// drain in time — so a caller that itself holds a read connection cannot deadlock. The pool lock is released
    /// before the writer is taken (the gate stays closed meanwhile), so the lock order never inverts with `write`.
    pub fn checkpoint_gate(&self, wait: Duration) -> Result<bool, StoreError> {
        let deadline = Instant::now().checked_add(wait);
        let mut st = self.readers.lock();
        st.gate = true;
        while st.free.len() < self.readers.size {
            let left = deadline.map(|d| d.saturating_duration_since(Instant::now()));
            if left.is_some_and(|l| l.is_zero()) {
                st.gate = false;
                drop(st);
                self.readers.cv.notify_all();
                return Ok(false);
            }
            st = match left {
                Some(l) => {
                    self.readers
                        .cv
                        .wait_timeout(st, l)
                        .unwrap_or_else(|p| p.into_inner())
                        .0
                }
                None => self.readers.cv.wait(st).unwrap_or_else(|p| p.into_inner()),
            };
        }
        drop(st);
        let r =
            self.with_writer(|w| w.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())));
        self.readers.lock().gate = false;
        self.readers.cv.notify_all();
        r.map(|_| true).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn open(path: &Path) -> Result<Db, StoreError> {
        Db::open(
            path,
            DbOptions {
                lock_wait: Duration::from_millis(200),
                ..Default::default()
            },
            |c| {
                c.execute_batch("CREATE TABLE IF NOT EXISTS t (x INTEGER) STRICT;")?;
                Ok(())
            },
        )
    }

    /// Re-measured 2026-10-25: agent databases never got a maintenance tick, so under busy readers the WAL grew without
    /// bound and reads slowed to seconds. The write path now runs the WAL gate itself.
    #[test]
    fn the_write_path_keeps_the_wal_bounded_with_busy_readers() {
        let tmp = tempfile::tempdir().unwrap();
        let limit = 256 * 1024;
        let db = Arc::new(
            Db::open(
                &tmp.path().join("w.db"),
                DbOptions {
                    lock_wait: Duration::from_millis(200),
                    wal_limit_bytes: limit,
                    readers: 2,
                    ..Default::default()
                },
                |c| {
                    c.execute_batch("CREATE TABLE IF NOT EXISTS t (x TEXT) STRICT;")?;
                    Ok(())
                },
            )
            .unwrap(),
        );
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let readers: Vec<_> = (0..2)
            .map(|_| {
                let (db, stop) = (db.clone(), stop.clone());
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        db.read(|c| -> Result<i64, StoreError> {
                            Ok(c.query_row("SELECT count(*) FROM t", [], |r| r.get(0))?)
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        let row = "x".repeat(400);
        let mut peak = 0;
        for _ in 0..4000 {
            db.write(|tx| -> Result<(), StoreError> {
                tx.execute("INSERT INTO t (x) VALUES (?1)", [&row])?;
                Ok(())
            })
            .unwrap();
            peak = peak.max(db.wal_bytes());
        }
        stop.store(true, Ordering::Relaxed);
        for r in readers {
            r.join().unwrap();
        }
        // ~1.6 MB written; without the gate the WAL keeps all of it (busy readers block passive checkpoints).
        assert!(
            peak < 6 * limit,
            "WAL peaked at {peak} bytes (limit {limit})"
        );
    }

    /// The gate never deadlocks a caller that holds a read: it gives up after its wait and reopens the pool.
    #[test]
    fn the_wal_gate_gives_up_when_a_read_is_held() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(open(&tmp.path().join("g.db")).unwrap());
        let held = db.clone();
        let reader = std::thread::spawn(move || {
            held.read(|_| -> Result<(), StoreError> {
                std::thread::sleep(Duration::from_millis(600));
                Ok(())
            })
            .unwrap();
        });
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            !db.checkpoint_gate(Duration::from_millis(100)).unwrap(),
            "gave up"
        );
        // The pool is open again: a read goes through.
        db.read(|c| -> Result<i64, StoreError> { Ok(c.query_row("SELECT 1", [], |r| r.get(0))?) })
            .unwrap();
        reader.join().unwrap();
        assert!(
            db.checkpoint_gate(Duration::from_secs(5)).unwrap(),
            "runs once reads drain"
        );
    }

    /// A read whose closure panics hands its connection back, its open transaction rolled back: the pool shrank for
    /// good instead, and the exclusive checkpoint, which waits for every reader, waited forever.
    #[test]
    fn a_panicking_read_returns_its_connection() {
        let tmp = tempfile::tempdir().unwrap();
        let db = open(&tmp.path().join("p.db")).unwrap();
        for _ in 0..db.readers.size {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                db.read(|c| -> Result<(), StoreError> {
                    c.execute_batch("BEGIN")?;
                    c.query_row("SELECT count(*) FROM t", [], |_| Ok(()))?;
                    panic!("a read that panics inside its transaction");
                })
            }));
            assert!(r.is_err());
        }
        assert!(
            db.checkpoint_gate(Duration::from_secs(2)).unwrap(),
            "every reader is back in the pool"
        );
        db.read(|c| -> Result<(), StoreError> {
            c.execute_batch("BEGIN")?;
            c.query_row("SELECT count(*) FROM t", [], |_| Ok(()))?;
            c.execute_batch("COMMIT")?;
            Ok(())
        })
        .unwrap();
        db.checkpoint_exclusive().unwrap();
    }

    #[test]
    fn second_open_in_same_process_is_locked_until_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("a.db");
        let db = open(&p).unwrap();
        assert!(matches!(open(&p), Err(StoreError::Locked(_))));
        drop(db);
        open(&p).unwrap();
    }

    #[test]
    fn writes_roll_back_on_error_and_readers_see_commits() {
        let tmp = tempfile::tempdir().unwrap();
        let db = open(&tmp.path().join("a.db")).unwrap();
        db.write(|tx| -> Result<(), StoreError> {
            tx.execute("INSERT INTO t VALUES (1)", [])?;
            Ok(())
        })
        .unwrap();
        let r: Result<(), StoreError> = db.write(|tx| {
            tx.execute("INSERT INTO t VALUES (2)", [])?;
            Err(StoreError::Invalid("boom".into()))
        });
        assert!(r.is_err());
        let n: i64 = db
            .read(|c| {
                c.query_row("SELECT count(*) FROM t", [], |r| r.get(0))
                    .map_err(StoreError::from)
            })
            .unwrap();
        assert_eq!(n, 1);
        let v: String = db
            .read(|c| {
                c.query_row("SELECT vec_version()", [], |r| r.get(0))
                    .map_err(StoreError::from)
            })
            .unwrap();
        assert!(v.starts_with('v'));
    }

    #[test]
    fn exclusive_checkpoint_waits_for_readers_and_truncates() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Arc::new(open(&tmp.path().join("a.db")).unwrap());
        db.write(|tx| -> Result<(), StoreError> {
            for i in 0..5000 {
                tx.execute("INSERT INTO t VALUES (?1)", [i])?;
            }
            Ok(())
        })
        .unwrap();
        let readers: Vec<_> = (0..8)
            .map(|_| {
                let db = db.clone();
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        let n: i64 = db
                            .read(|c| {
                                c.query_row("SELECT count(*) FROM t", [], |r| r.get(0))
                                    .map_err(StoreError::from)
                            })
                            .unwrap();
                        assert_eq!(n, 5000);
                    }
                })
            })
            .collect();
        db.checkpoint_exclusive().unwrap();
        for r in readers {
            r.join().unwrap();
        }
        db.checkpoint_exclusive().unwrap();
        assert_eq!(db.wal_bytes(), 0);
    }
}
