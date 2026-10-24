//! One SQLite database: a serialized writer connection, a fair pool of read connections (WAL snapshots) and a
//! process lock file. Concurrency model from ADR-50 §8: `BEGIN IMMEDIATE` writes, condvar-queued readers with a
//! large mmap, and a WAL gate that pauses reads for a TRUNCATE checkpoint when the WAL grows past a limit.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
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

pub struct Db {
    path: PathBuf,
    writer: Mutex<Connection>,
    readers: ReaderPool,
    opts: DbOptions,
    _lock: File,
}

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
        let c = self.readers.acquire();
        let r = f(&c);
        self.readers.release(c);
        r
    }

    /// Runs `f` inside `BEGIN IMMEDIATE` on the writer; commits on `Ok`, rolls back on `Err`.
    pub fn write<T, E: From<StoreError>>(
        &self,
        f: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
    ) -> Result<T, E> {
        let mut w = self.writer.lock().unwrap_or_else(|p| p.into_inner());
        let tx = w
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| E::from(StoreError::from(e)))?;
        let v = f(&tx)?;
        tx.commit().map_err(|e| E::from(StoreError::from(e)))?;
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
    /// in-flight ones and TRUNCATE (the WAL gate). Returns the WAL size afterwards.
    pub fn maintain(&self) -> Result<u64, StoreError> {
        self.with_writer(|w| w.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(())))?;
        if self.wal_bytes() > self.opts.wal_limit_bytes {
            self.checkpoint_exclusive()?;
        }
        Ok(self.wal_bytes())
    }

    /// TRUNCATE checkpoint with all readers parked.
    pub fn checkpoint_exclusive(&self) -> Result<(), StoreError> {
        let mut st = self.readers.lock();
        st.gate = true;
        while st.free.len() < self.readers.size {
            st = self.readers.cv.wait(st).unwrap_or_else(|p| p.into_inner());
        }
        let r =
            self.with_writer(|w| w.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())));
        st.gate = false;
        drop(st);
        self.readers.cv.notify_all();
        r.map_err(Into::into)
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
