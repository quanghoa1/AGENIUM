//! Backups (decision 23, ADR-46 as amended by ADR-49).
//!
//! * **Full** = a consistent snapshot image of each database (SQLite online-backup API), zstd-compressed, with
//!   sha256 and a per-table logical content hash.
//! * **Incremental** (core only) = zstd JSONL relative to a full: append-only tables (`event`, `intent`,
//!   `run_log`) carry only rows past the base's cut; the small mutable tables are dumped whole. Restoring =
//!   base snapshot + apply. Agent databases are snapshotted in full in every set (their incremental form is M1.5).
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

pub const MANIFEST: &str = "manifest.json";
const ZSTD_LEVEL: i32 = 3;

/// Tables whose rows are never updated or deleted: incremental backups carry only new rows (by rowid).
const APPEND_ONLY: &[&str] = &["event", "intent", "run_log"];

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

fn sha256_file(path: &Path) -> Result<String, StoreError> {
    let mut f = File::open(path).map_err(|e| StoreError::io(path, e))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(|e| StoreError::io(path, e))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn compress(src: &Path, dst: &Path) -> Result<(), StoreError> {
    let input = File::open(src).map_err(|e| StoreError::io(src, e))?;
    let out = File::create(dst).map_err(|e| StoreError::io(dst, e))?;
    let mut enc =
        zstd::Encoder::new(BufWriter::new(out), ZSTD_LEVEL).map_err(|e| StoreError::io(dst, e))?;
    std::io::copy(&mut BufReader::new(input), &mut enc).map_err(|e| StoreError::io(dst, e))?;
    let w = enc.finish().map_err(|e| StoreError::io(dst, e))?;
    w.into_inner()
        .map_err(|e| StoreError::io(dst, e.into_error()))?
        .sync_all()
        .map_err(|e| StoreError::io(dst, e))
}

fn decompress(src: &Path, dst: &Path) -> Result<(), StoreError> {
    let input = File::open(src).map_err(|e| StoreError::io(src, e))?;
    let mut dec = zstd::Decoder::new(input).map_err(|e| StoreError::io(src, e))?;
    let mut out = BufWriter::new(File::create(dst).map_err(|e| StoreError::io(dst, e))?);
    std::io::copy(&mut dec, &mut out).map_err(|e| StoreError::io(dst, e))?;
    out.into_inner()
        .map_err(|e| StoreError::io(dst, e.into_error()))?
        .sync_all()
        .map_err(|e| StoreError::io(dst, e))
}

/// Copies a consistent image of `src` into a new database file `dst` (rollback journal, not WAL).
pub fn snapshot(src: &Connection, dst: &Path) -> Result<(), StoreError> {
    register_sqlite_vec();
    if dst.exists() {
        fs::remove_file(dst).map_err(|e| StoreError::io(dst, e))?;
    }
    let mut out = Connection::open(dst)?;
    {
        let b = Backup::new(src, &mut out)?;
        b.run_to_completion(1000, std::time::Duration::ZERO, None)?;
    }
    out.pragma_update(None, "journal_mode", "DELETE")?;
    Ok(())
}

/// Names of virtual tables and their shadow tables (excluded from logical hashes; rebuilt by SQLite/extensions).
fn derived_tables(c: &Connection) -> Result<BTreeSet<String>, StoreError> {
    let mut st = c.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND sql LIKE 'CREATE VIRTUAL TABLE%'",
    )?;
    let vt: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let mut st = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
    let all: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(all
        .into_iter()
        .filter(|t| vt.iter().any(|v| t == v || t.starts_with(&format!("{v}_"))))
        .collect())
}

/// Logical tables (user tables that are not derived and not internal).
pub fn logical_tables(c: &Connection) -> Result<Vec<String>, StoreError> {
    let derived = derived_tables(c)?;
    let mut st = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")?;
    let all: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(all.into_iter().filter(|t| !derived.contains(t)).collect())
}

/// `true` when the table has a single-column `INTEGER PRIMARY KEY` (an alias of rowid).
fn has_rowid_alias(c: &Connection, table: &str) -> Result<bool, StoreError> {
    let mut st = c.prepare(&format!(
        "SELECT type, pk FROM pragma_table_info({})",
        quote_literal(table)
    ))?;
    let pks: Vec<(String, i64)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .filter_map(|r| r.ok())
        .filter(|(_, pk)| *pk > 0)
        .collect();
    Ok(pks.len() == 1 && pks[0].0.eq_ignore_ascii_case("INTEGER"))
}

fn quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// sha256 per logical table over every row (ordered by rowid), type-tagged.
pub fn content_hash(c: &Connection) -> Result<BTreeMap<String, String>, StoreError> {
    let mut out = BTreeMap::new();
    for t in logical_tables(c)? {
        let mut h = Sha256::new();
        let mut st = c.prepare(&format!("SELECT * FROM {} ORDER BY rowid", quote_ident(&t)))?;
        let n = st.column_count();
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            for i in 0..n {
                match r.get_ref(i)? {
                    ValueRef::Null => h.update([0u8]),
                    ValueRef::Integer(v) => {
                        h.update([1u8]);
                        h.update(v.to_le_bytes());
                    }
                    ValueRef::Real(v) => {
                        h.update([2u8]);
                        h.update(v.to_le_bytes());
                    }
                    ValueRef::Text(b) => {
                        h.update([3u8]);
                        h.update((b.len() as u64).to_le_bytes());
                        h.update(b);
                    }
                    ValueRef::Blob(b) => {
                        h.update([4u8]);
                        h.update((b.len() as u64).to_le_bytes());
                        h.update(b);
                    }
                }
            }
            h.update([0xffu8]);
        }
        out.insert(t, hex(&h.finalize()));
    }
    Ok(out)
}

fn schema_version(c: &Connection) -> Result<u32, StoreError> {
    Ok(c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as u32)
}

fn stamp(now: DateTime<Utc>) -> String {
    now.format("%Y%m%dT%H%M%S%3fZ").to_string()
}

/// Snapshot + compress one database into `dir/<file>`; returns its manifest entry.
fn snapshot_entry(c: &Connection, name: &str, dir: &Path) -> Result<DbEntry, StoreError> {
    let file = format!("{}.db.zst", name.replace('/', "__"));
    let tmp = dir.join(format!("{file}.tmp-db"));
    snapshot(c, &tmp)?;
    let (tables, version) = {
        let snap = Connection::open_with_flags(&tmp, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        (content_hash(&snap)?, schema_version(&snap)?)
    };
    let dst = dir.join(&file);
    compress(&tmp, &dst)?;
    fs::remove_file(&tmp).map_err(|e| StoreError::io(&tmp, e))?;
    let bytes = fs::metadata(&dst)
        .map_err(|e| StoreError::io(&dst, e))?
        .len();
    Ok(DbEntry {
        name: name.into(),
        file,
        sha256: sha256_file(&dst)?,
        bytes,
        schema_version: version,
        tables,
    })
}

fn append_cuts(c: &Connection) -> Result<BTreeMap<String, i64>, StoreError> {
    let mut cuts = BTreeMap::new();
    for t in APPEND_ONLY {
        let exists: bool = c.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
            [t],
            |r| r.get(0),
        )?;
        if exists {
            let n: i64 = c.query_row(
                &format!("SELECT COALESCE(MAX(rowid), 0) FROM {t}"),
                [],
                |r| r.get(0),
            )?;
            cuts.insert((*t).to_owned(), n);
        }
    }
    Ok(cuts)
}

fn write_manifest(dir: &Path, m: &Manifest) -> Result<(), StoreError> {
    let p = dir.join(MANIFEST);
    fs::write(&p, serde_json::to_vec_pretty(m)?).map_err(|e| StoreError::io(&p, e))
}

pub fn read_manifest(set_dir: &Path) -> Result<Manifest, StoreError> {
    let p = set_dir.join(MANIFEST);
    let b = fs::read(&p).map_err(|e| StoreError::io(&p, e))?;
    Ok(serde_json::from_slice(&b)?)
}

/// Creates a full backup set under `backups_dir` from `dbs` (`core` first by convention).
/// A database to back up: an open [`Db`] (consistent snapshot through its reader pool) or a closed database
/// file (opened read-only just for the snapshot).
pub enum Source<'a> {
    Open(&'a Db),
    File(PathBuf),
}

pub fn create_full(
    backups_dir: &Path,
    dbs: &[(&str, &Db)],
    now: DateTime<Utc>,
) -> Result<(PathBuf, Manifest), StoreError> {
    let sources: Vec<(String, Source<'_>)> = dbs
        .iter()
        .map(|(n, d)| (n.to_string(), Source::Open(d)))
        .collect();
    create_full_from(backups_dir, &sources, now)
}

/// Like [`create_full`] with databases that may not be open in this process.
pub fn create_full_from(
    backups_dir: &Path,
    dbs: &[(String, Source<'_>)],
    now: DateTime<Utc>,
) -> Result<(PathBuf, Manifest), StoreError> {
    let id = format!("{}-full", stamp(now));
    let dir = backups_dir.join(&id);
    let tmp_dir = backups_dir.join(format!(".{id}.partial"));
    fs::create_dir_all(&tmp_dir).map_err(|e| StoreError::io(&tmp_dir, e))?;
    let mut entries = Vec::new();
    let mut cuts = BTreeMap::new();
    for (name, src) in dbs {
        let mut f = |c: &Connection| -> Result<DbEntry, StoreError> {
            if name == "core" {
                cuts = append_cuts(c)?;
            }
            snapshot_entry(c, name, &tmp_dir)
        };
        let e = match src {
            Source::Open(db) => db.read(|c| f(c))?,
            Source::File(p) => {
                register_sqlite_vec();
                let c = Connection::open_with_flags(p, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                f(&c)?
            }
        };
        entries.push(e);
    }
    let m = Manifest {
        id: id.clone(),
        kind: BackupKind::Full,
        created_at: now.to_rfc3339(),
        base: None,
        cuts,
        dbs: entries,
    };
    write_manifest(&tmp_dir, &m)?;
    fs::rename(&tmp_dir, &dir).map_err(|e| StoreError::io(&dir, e))?;
    Ok((dir, m))
}

fn value_to_json(v: ValueRef<'_>) -> serde_json::Value {
    use serde_json::json;
    match v {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!({ "r": f }),
        ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
        ValueRef::Blob(b) => json!({ "b": hex(b) }),
    }
}

fn json_to_value(v: &serde_json::Value) -> Result<Value, StoreError> {
    Ok(match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Number(n) => Value::Integer(
            n.as_i64()
                .ok_or_else(|| StoreError::Corrupt("bad integer".into()))?,
        ),
        serde_json::Value::String(s) => Value::Text(s.clone()),
        serde_json::Value::Object(o) if o.contains_key("r") => {
            Value::Real(o["r"].as_f64().unwrap_or_default())
        }
        serde_json::Value::Object(o) if o.contains_key("b") => {
            let h = o["b"].as_str().unwrap_or_default();
            let bytes = (0..h.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&h[i..i + 2], 16))
                .collect::<Result<Vec<u8>, _>>()
                .map_err(|_| StoreError::Corrupt("bad blob".into()))?;
            Value::Blob(bytes)
        }
        other => {
            return Err(StoreError::Corrupt(format!(
                "unexpected JSON value {other}"
            )))
        }
    })
}

/// Creates an incremental set for `core` relative to the full set `base_dir`.
pub fn create_incremental(
    backups_dir: &Path,
    base_dir: &Path,
    core: &Db,
    now: DateTime<Utc>,
) -> Result<(PathBuf, Manifest), StoreError> {
    let base = read_manifest(base_dir)?;
    if base.kind != BackupKind::Full {
        return Err(StoreError::Invalid(
            "incremental base must be a full set".into(),
        ));
    }
    let id = format!("{}-incr", stamp(now));
    let dir = backups_dir.join(&id);
    let tmp_dir = backups_dir.join(format!(".{id}.partial"));
    fs::create_dir_all(&tmp_dir).map_err(|e| StoreError::io(&tmp_dir, e))?;
    let file = "core.jsonl.zst";
    let path = tmp_dir.join(file);
    let (cuts, tables, version) = core.read(|c| -> Result<_, StoreError> {
        // One read transaction = one consistent snapshot for the whole dump.
        c.execute_batch("BEGIN")?;
        let r = (|| {
            let out = File::create(&path).map_err(|e| StoreError::io(&path, e))?;
            let mut enc = zstd::Encoder::new(BufWriter::new(out), ZSTD_LEVEL).map_err(|e| StoreError::io(&path, e))?;
            for t in logical_tables(c)? {
                let cut = base.cuts.get(&t).copied();
                // Keep rowids (content hashes order by rowid) unless an INTEGER PRIMARY KEY already aliases it.
                let explicit_rowid = !has_rowid_alias(c, &t)?;
                let sel = if explicit_rowid { "rowid, *" } else { "*" };
                let sql = match cut {
                    Some(n) => format!("SELECT {sel} FROM {} WHERE rowid > {n} ORDER BY rowid", quote_ident(&t)),
                    None => format!("SELECT {sel} FROM {} ORDER BY rowid", quote_ident(&t)),
                };
                let mut st = c.prepare(&sql)?;
                let cols: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
                let header = serde_json::json!({ "table": t, "cols": cols, "mode": if cut.is_some() { "append" } else { "replace" } });
                writeln!(enc, "{header}").map_err(|e| StoreError::io(&path, e))?;
                let n = st.column_count();
                let mut rows = st.query([])?;
                while let Some(r) = rows.next()? {
                    let vals: Vec<serde_json::Value> = (0..n).map(|i| r.get_ref(i).map(value_to_json)).collect::<Result<_, _>>()?;
                    writeln!(enc, "{}", serde_json::Value::Array(vals)).map_err(|e| StoreError::io(&path, e))?;
                }
            }
            let w = enc.finish().map_err(|e| StoreError::io(&path, e))?;
            w.into_inner().map_err(|e| StoreError::io(&path, e.into_error()))?.sync_all().map_err(|e| StoreError::io(&path, e))?;
            Ok((append_cuts(c)?, content_hash(c)?, schema_version(c)?))
        })();
        c.execute_batch("COMMIT")?;
        r
    })?;
    let bytes = fs::metadata(&path)
        .map_err(|e| StoreError::io(&path, e))?
        .len();
    let entry = DbEntry {
        name: "core".into(),
        file: file.into(),
        sha256: sha256_file(&path)?,
        bytes,
        schema_version: version,
        tables,
    };
    let m = Manifest {
        id: id.clone(),
        kind: BackupKind::Incremental,
        created_at: now.to_rfc3339(),
        base: Some(base.id),
        cuts,
        dbs: vec![entry],
    };
    write_manifest(&tmp_dir, &m)?;
    fs::rename(&tmp_dir, &dir).map_err(|e| StoreError::io(&dir, e))?;
    Ok((dir, m))
}

fn apply_incremental(db_path: &Path, jsonl_zst: &Path) -> Result<(), StoreError> {
    register_sqlite_vec();
    let c = Connection::open(db_path)?;
    c.pragma_update(None, "foreign_keys", "OFF")?;
    c.execute_batch("BEGIN")?;
    let r = (|| -> Result<(), StoreError> {
        let f = File::open(jsonl_zst).map_err(|e| StoreError::io(jsonl_zst, e))?;
        let dec = zstd::Decoder::new(f).map_err(|e| StoreError::io(jsonl_zst, e))?;
        let mut insert_sql = String::new();
        for line in BufReader::new(dec).lines() {
            let line = line.map_err(|e| StoreError::io(jsonl_zst, e))?;
            let v: serde_json::Value = serde_json::from_str(&line)?;
            if let Some(t) = v.get("table").and_then(|t| t.as_str()) {
                let cols: Vec<String> = v["cols"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c.as_str().map(quote_ident))
                    .collect();
                if v["mode"] == "replace" {
                    c.execute(&format!("DELETE FROM {}", quote_ident(t)), [])?;
                }
                let placeholders = vec!["?"; cols.len()].join(", ");
                insert_sql = format!(
                    "INSERT OR REPLACE INTO {} ({}) VALUES ({placeholders})",
                    quote_ident(t),
                    cols.join(", ")
                );
                continue;
            }
            let vals: Vec<Value> = v
                .as_array()
                .into_iter()
                .flatten()
                .map(json_to_value)
                .collect::<Result<_, _>>()?;
            c.execute(&insert_sql, rusqlite::params_from_iter(vals))?;
        }
        Ok(())
    })();
    match r {
        Ok(()) => c.execute_batch("COMMIT")?,
        Err(e) => {
            c.execute_batch("ROLLBACK")?;
            return Err(e);
        }
    }
    c.pragma_update(None, "foreign_keys", "ON")?;
    let bad: i64 = c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
        r.get(0)
    })?;
    if bad > 0 {
        return Err(StoreError::Corrupt(format!(
            "{bad} foreign key violations after applying incremental backup"
        )));
    }
    Ok(())
}

fn verify_entry(set_dir: &Path, e: &DbEntry) -> Result<(), StoreError> {
    let p = set_dir.join(&e.file);
    let sha = sha256_file(&p)?;
    if sha != e.sha256 {
        return Err(StoreError::Corrupt(format!(
            "{}: sha256 mismatch",
            p.display()
        )));
    }
    Ok(())
}

/// Restores a backup set into `target_db_dir` (the `db/` directory of a new root): `core.db` and
/// `agents/<id>.db`. Fails if a target file exists. Verifies sha256 of every file and the logical content hash
/// of every restored database.
pub fn restore(
    backups_dir: &Path,
    set_id: &str,
    target_db_dir: &Path,
) -> Result<Manifest, StoreError> {
    let set_dir = backups_dir.join(set_id);
    let m = read_manifest(&set_dir)?;
    let (full_dir, full) = match m.kind {
        BackupKind::Full => (set_dir.clone(), m.clone()),
        BackupKind::Incremental => {
            let base = m
                .base
                .clone()
                .ok_or_else(|| StoreError::Corrupt("incremental without base".into()))?;
            let d = backups_dir.join(&base);
            let fm = read_manifest(&d)?;
            (d, fm)
        }
    };
    for e in &full.dbs {
        verify_entry(&full_dir, e)?;
        let dst = target_db_dir.join(format!("{}.db", e.name));
        if dst.exists() {
            return Err(StoreError::Conflict(format!(
                "{} already exists",
                dst.display()
            )));
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|er| StoreError::io(parent, er))?;
        }
        decompress(&full_dir.join(&e.file), &dst)?;
    }
    let mut expected: BTreeMap<String, BTreeMap<String, String>> = full
        .dbs
        .iter()
        .map(|e| (e.name.clone(), e.tables.clone()))
        .collect();
    if m.kind == BackupKind::Incremental {
        for e in &m.dbs {
            verify_entry(&set_dir, e)?;
            apply_incremental(
                &target_db_dir.join(format!("{}.db", e.name)),
                &set_dir.join(&e.file),
            )?;
            expected.insert(e.name.clone(), e.tables.clone());
        }
    }
    for (name, tables) in &expected {
        let c = Connection::open_with_flags(
            target_db_dir.join(format!("{name}.db")),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let got = content_hash(&c)?;
        if &got != tables {
            let diff: Vec<_> = tables
                .keys()
                .filter(|t| got.get(*t) != tables.get(*t))
                .cloned()
                .collect();
            return Err(StoreError::Corrupt(format!(
                "{name}: content hash mismatch in {diff:?}"
            )));
        }
    }
    Ok(m)
}

/// Pre-migrate snapshot of an open connection: `<dir>/pre-migrate/<name>-v<from>-<stamp>.db.zst`; keeps the
/// newest 3 per name.
pub fn pre_migrate_snapshot(
    c: &Connection,
    dir: &Path,
    name: &str,
    from: u32,
) -> Result<PathBuf, StoreError> {
    let d = dir.join("pre-migrate");
    fs::create_dir_all(&d).map_err(|e| StoreError::io(&d, e))?;
    let tmp = d.join(format!("{name}.tmp-db"));
    snapshot(c, &tmp)?;
    let dst = d.join(format!("{name}-v{from}-{}.db.zst", stamp(Utc::now())));
    compress(&tmp, &dst)?;
    fs::remove_file(&tmp).map_err(|e| StoreError::io(&tmp, e))?;
    let prefix = format!("{name}-v");
    let mut files: Vec<PathBuf> = fs::read_dir(&d)
        .map_err(|e| StoreError::io(&d, e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".db.zst"))
        })
        .collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_owned()));
    while files.len() > 3 {
        let old = files.remove(0);
        fs::remove_file(&old).map_err(|e| StoreError::io(&old, e))?;
    }
    Ok(dst)
}

#[derive(Debug, Clone, Copy)]
pub struct Retention {
    pub hourly: usize,
    pub daily: usize,
    pub weekly: usize,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            hourly: 24,
            daily: 7,
            weekly: 4,
        }
    }
}

/// Lists backup sets (oldest first).
pub fn list_sets(backups_dir: &Path) -> Result<Vec<Manifest>, StoreError> {
    let mut out = Vec::new();
    let rd = match fs::read_dir(backups_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(StoreError::io(backups_dir, e)),
    };
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        if p.join(MANIFEST).is_file() {
            out.push(read_manifest(&p)?);
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Retention 24 hourly / 7 daily / 4 weekly: keeps the newest set of each of the newest N hours/days/ISO weeks,
/// never deletes a full set that a kept incremental depends on. Returns removed set ids.
pub fn prune(backups_dir: &Path, r: Retention) -> Result<Vec<String>, StoreError> {
    let sets = list_sets(backups_dir)?;
    let parse_time = |m: &Manifest| {
        DateTime::parse_from_rfc3339(&m.created_at)
            .map(|d| d.with_timezone(&Utc))
            .ok()
    };
    let mut keep: BTreeSet<String> = BTreeSet::new();
    let bucket = |key: &dyn Fn(DateTime<Utc>) -> String, n: usize, keep: &mut BTreeSet<String>| {
        let mut seen = BTreeSet::new();
        for m in sets.iter().rev() {
            if let Some(t) = parse_time(m) {
                let k = key(t);
                if seen.len() < n && seen.insert(k) {
                    keep.insert(m.id.clone());
                }
            }
        }
    };
    bucket(&|t| t.format("%Y%m%d%H").to_string(), r.hourly, &mut keep);
    bucket(&|t| t.format("%Y%m%d").to_string(), r.daily, &mut keep);
    bucket(
        &|t| format!("{}-{}", t.iso_week().year(), t.iso_week().week()),
        r.weekly,
        &mut keep,
    );
    let bases: Vec<String> = sets
        .iter()
        .filter(|m| keep.contains(&m.id))
        .filter_map(|m| m.base.clone())
        .collect();
    keep.extend(bases);
    let mut removed = Vec::new();
    for m in &sets {
        if !keep.contains(&m.id) {
            let d = backups_dir.join(&m.id);
            fs::remove_dir_all(&d).map_err(|e| StoreError::io(&d, e))?;
            removed.push(m.id.clone());
        }
    }
    Ok(removed)
}
