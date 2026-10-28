//! Restoring a set (verified, increments applied) and the snapshot before a migration.

use super::*;

pub(super) fn apply_incremental(db_path: &Path, jsonl_zst: &Path) -> Result<(), StoreError> {
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

pub(super) fn verify_entry(set_dir: &Path, e: &DbEntry) -> Result<(), StoreError> {
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
