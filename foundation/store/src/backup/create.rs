//! Backup sets: full and incremental, with their manifest.

use super::*;

pub(super) fn stamp(now: DateTime<Utc>) -> String {
    now.format("%Y%m%dT%H%M%S%3fZ").to_string()
}

/// Snapshot + compress one database into `dir/<file>`; returns its manifest entry.
pub(super) fn snapshot_entry(
    c: &Connection,
    name: &str,
    dir: &Path,
) -> Result<DbEntry, StoreError> {
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

pub(super) fn append_cuts(c: &Connection) -> Result<BTreeMap<String, i64>, StoreError> {
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

pub(super) fn write_manifest(dir: &Path, m: &Manifest) -> Result<(), StoreError> {
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
        // The cuts and the image come from the same snapshot.
        let mut f = |c: &Connection| -> Result<DbEntry, StoreError> {
            in_read_tx(c, |c| {
                if name == "core" {
                    cuts = append_cuts(c)?;
                }
                snapshot_entry(c, name, &tmp_dir)
            })
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
        agents_in_full: false,
    };
    write_manifest(&tmp_dir, &m)?;
    fs::rename(&tmp_dir, &dir).map_err(|e| StoreError::io(&dir, e))?;
    Ok((dir, m))
}

pub(crate) fn value_to_json(v: ValueRef<'_>) -> serde_json::Value {
    use serde_json::json;
    match v {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!({ "r": f }),
        ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
        ValueRef::Blob(b) => json!({ "b": hex(b) }),
    }
}

pub(crate) fn json_to_value(v: &serde_json::Value) -> Result<Value, StoreError> {
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
            Value::Blob(unhex(h).ok_or_else(|| StoreError::Corrupt("bad blob".into()))?)
        }
        other => {
            return Err(StoreError::Corrupt(format!(
                "unexpected JSON value {other}"
            )))
        }
    })
}

/// The schema version `core` is at now: a set whose base was taken at another one must be a full set.
pub fn schema_version_of(db: &Db) -> Result<u32, StoreError> {
    db.read(schema_version)
}

/// Creates an incremental set for `core` relative to the full set `base_dir`, with `agents` (every agent database)
/// snapshotted in full.
pub fn create_incremental(
    backups_dir: &Path,
    base_dir: &Path,
    core: &Db,
    agents: &[(String, Source<'_>)],
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
                // A full set made before `run_log` left APPEND_ONLY still has a cut for it: dump that table whole.
                let cut = base.cuts.get(&t).copied().filter(|_| APPEND_ONLY.contains(&t.as_str()));
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
    let mut dbs = vec![entry];
    for (name, src) in agents {
        let f = |c: &Connection| in_read_tx(c, |c| snapshot_entry(c, name, &tmp_dir));
        dbs.push(match src {
            Source::Open(db) => db.read(f)?,
            Source::File(p) => {
                register_sqlite_vec();
                f(&Connection::open_with_flags(
                    p,
                    OpenFlags::SQLITE_OPEN_READ_ONLY,
                )?)?
            }
        });
    }
    let m = Manifest {
        id: id.clone(),
        kind: BackupKind::Incremental,
        created_at: now.to_rfc3339(),
        base: Some(base.id),
        cuts,
        dbs,
        agents_in_full: true,
    };
    write_manifest(&tmp_dir, &m)?;
    fs::rename(&tmp_dir, &dir).map_err(|e| StoreError::io(&dir, e))?;
    Ok((dir, m))
}
