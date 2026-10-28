//! Backups (decision 23, ADR-46 as amended by ADR-49).
//!
//! * **Full** = a consistent snapshot image of each database (SQLite online-backup API), zstd-compressed, with
//!   sha256 and a per-table logical content hash.
//! * **Incremental** (core only) = zstd JSONL relative to a full: append-only tables (`event`, `intent`) carry
//!   only rows past the base's cut; the mutable tables are dumped whole (`run_log` too: compaction deletes its rows,
//!   ADR-76). Restoring = base snapshot + apply. Agent databases are snapshotted in full in every set (their
//!   incremental form is M1.5).
//! * Retention: 24 hourly / 7 daily / 4 weekly sets, plus the 3 newest pre-migrate snapshots.
//! * Never contains secrets (they are not in any database) and never duplicates git content.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Utc};
use rusqlite::backup::Backup;
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::{register_sqlite_vec, Db};
use crate::StoreError;

mod snapshot;

pub use create::*;
pub use restore::*;
pub use retention::*;
pub use snapshot::*;
mod create;
mod restore;
mod retention;

pub const MANIFEST: &str = "manifest.json";
const ZSTD_LEVEL: i32 = 3;

/// Tables whose rows are never updated or deleted: incremental backups carry only new rows (by rowid). Not `run_log`:
/// `runs::compact_logs` deletes rows of old runs and adds a marker, so an incremental that carried only rows past the
/// cut restored the deleted rows and failed its content hash (and a reused rowid could fall below the cut).
const APPEND_ONLY: &[&str] = &["event", "intent"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackupKind {
    Full,
    Incremental,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbEntry {
    /// Logical name: `core` or `agents/<agent-id>`.
    pub name: String,
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
    pub schema_version: u32,
    /// Logical content hash per table (virtual tables and their shadow tables excluded).
    pub tables: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub kind: BackupKind,
    pub created_at: String,
    /// Incremental: id of the full set it applies to.
    pub base: Option<String>,
    /// Max rowid per append-only table of `core` at backup time (the next incremental's cut).
    pub cuts: BTreeMap<String, i64>,
    pub dbs: Vec<DbEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A malformed blob in an import or incremental file is an error, never a panic (odd length; a non-ASCII
    /// character that two-byte slicing split; a sign `from_str_radix` accepted).
    #[test]
    fn a_malformed_blob_is_an_error_not_a_panic() {
        let blob = |h: &str| json_to_value(&serde_json::json!({ "b": h }));
        assert_eq!(blob("00ff").unwrap(), Value::Blob(vec![0, 255]));
        assert_eq!(blob("").unwrap(), Value::Blob(vec![]));
        for bad in ["abc", "a\u{e9}a", "+f", "zz"] {
            assert!(matches!(blob(bad), Err(StoreError::Corrupt(_))), "{bad:?}");
        }
    }

    /// A snapshot of a database under steady writes is one pass, never restarted. Without one read transaction around
    /// it, the stepped backup started over from the first page whenever a step saw new commits, and only got through
    /// when a whole pass ran between two restarts (measured: 12 000 pages in 1000-page steps took 17 s, recopying the
    /// first 1000 pages again and again; on this database dozens of restarts in 25 ms).
    #[test]
    fn a_snapshot_under_steady_writes_never_restarts() {
        use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        static LAST: AtomicI64 = AtomicI64::new(i64::MAX);
        static RESTARTS: AtomicI64 = AtomicI64::new(0);
        fn watch(p: rusqlite::backup::Progress) {
            let remaining = i64::from(p.remaining);
            if remaining > LAST.swap(remaining, Ordering::SeqCst) {
                RESTARTS.fetch_add(1, Ordering::SeqCst);
            }
        }

        let tmp = tempfile::tempdir().unwrap();
        // No write-path WAL gate: it pauses the writer once the WAL is large, which let a restarting backup through.
        let opts = crate::db::DbOptions {
            synchronous_full: false,
            wal_limit_bytes: u64::MAX,
            ..Default::default()
        };
        let db = Arc::new(
            Db::open(&tmp.path().join("busy.db"), opts, |c| {
                c.execute_batch("CREATE TABLE t (x BLOB) STRICT;")?;
                Ok(())
            })
            .unwrap(),
        );
        // One row per page: ~2000 pages, 400 steps of 5 pages per pass.
        db.write(|tx| -> Result<(), StoreError> {
            for _ in 0..2000 {
                tx.execute("INSERT INTO t (x) VALUES (zeroblob(3000))", [])?;
            }
            Ok(())
        })
        .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let writer = {
            let (db, stop) = (db.clone(), stop.clone());
            std::thread::spawn(move || {
                let start = Instant::now();
                while !stop.load(Ordering::Relaxed) {
                    if start.elapsed() > Duration::from_secs(10) {
                        return false; // stopped writing before the snapshot finished
                    }
                    db.write(|tx| -> Result<(), StoreError> {
                        tx.execute("INSERT INTO t (x) VALUES (x'00')", [])?;
                        Ok(())
                    })
                    .unwrap();
                }
                true
            })
        };
        std::thread::sleep(Duration::from_millis(50));
        let copy = tmp.path().join("copy.db");
        db.read(|c| snapshot_in_steps(c, &copy, 5, Some(watch)))
            .unwrap();
        stop.store(true, Ordering::Relaxed);
        assert!(writer.join().unwrap(), "the writes went on throughout");
        assert_eq!(RESTARTS.load(Ordering::SeqCst), 0, "the backup restarted");
        let n: i64 = Connection::open(&copy)
            .unwrap()
            .query_row("SELECT count(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert!(n >= 2000, "{n}");
    }
}
