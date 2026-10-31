//! Which sets are kept: listing and pruning.

use super::*;

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
        // A set being written (`.<id>.partial`, its manifest written before the rename) or one whose rename failed
        // is not a set: listed under its manifest's id, its folder was missing for `prune` and as a base (audit L6,
        // 2026-10-31).
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !p.join(MANIFEST).is_file() {
            continue;
        }
        let m = read_manifest(&p)?;
        if m.id == name {
            out.push(m);
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
            match fs::remove_dir_all(&d) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(StoreError::io(&d, e)),
            }
            removed.push(m.id.clone());
        }
    }
    // Sets left half-written by a process that died (or whose rename failed) a day ago or more.
    if let Ok(rd) = fs::read_dir(backups_dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let stale = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > std::time::Duration::from_secs(24 * 3600));
            if name.starts_with('.') && name.ends_with(".partial") && stale {
                let _ = fs::remove_dir_all(e.path());
            }
        }
    }
    Ok(removed)
}
