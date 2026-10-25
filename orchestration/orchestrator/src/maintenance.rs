//! Periodic maintenance: WAL checkpoints and scheduled backups (decision 23: hourly incremental, daily full,
//! retention 24/7/4) of `core.db` and every agent database.

use std::path::{Path, PathBuf};

use agenium_store::backup::{self, BackupKind, Manifest, Retention, Source};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::app::App;
use crate::{OrchError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupRequest {
    /// Full when the newest full is a day old (or missing), else incremental when the newest set is an hour old.
    Scheduled,
    Full,
    Incremental,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MaintenanceReport {
    pub wal_bytes: u64,
    pub backup: Option<String>,
    pub pruned: Vec<String>,
    /// Runs whose logs were compacted this tick (ADR-76).
    pub logs_compacted: u64,
}

fn age_hours(m: &Manifest, now: DateTime<Utc>) -> f64 {
    DateTime::parse_from_rfc3339(&m.created_at)
        .map(|t| (now - t.with_timezone(&Utc)).num_seconds() as f64 / 3600.0)
        .unwrap_or(f64::MAX)
}

impl App {
    pub fn backups_dir(&self) -> PathBuf {
        let d = PathBuf::from(&self.config().paths.backups_dir);
        if d.is_absolute() {
            d
        } else {
            self.layout.root.join(d)
        }
    }

    fn agent_db_files(&self) -> Result<Vec<(String, PathBuf)>> {
        let dir = self.layout.agent_dbs();
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.filter_map(|e| e.ok()) {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) == Some("db") {
                    let stem = p
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_owned();
                    out.push((format!("agents/{stem}"), p));
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// Takes a backup now. Returns `None` when a scheduled backup is not due.
    pub fn backup(&self, req: BackupRequest, now: DateTime<Utc>) -> Result<Option<Manifest>> {
        let dir = self.backups_dir();
        let sets = backup::list_sets(&dir)?;
        let last_full = sets.iter().rev().find(|m| m.kind == BackupKind::Full);
        let full_due = last_full.is_none_or(|m| age_hours(m, now) >= 24.0);
        let any_due = sets.last().is_none_or(|m| age_hours(m, now) >= 1.0);
        let kind = match req {
            BackupRequest::Full => BackupKind::Full,
            BackupRequest::Incremental if last_full.is_some() => BackupKind::Incremental,
            BackupRequest::Incremental => BackupKind::Full,
            BackupRequest::Scheduled if full_due => BackupKind::Full,
            BackupRequest::Scheduled if any_due => BackupKind::Incremental,
            BackupRequest::Scheduled => return Ok(None),
        };
        let m = match kind {
            BackupKind::Full => {
                let open = self.open_agent_stores();
                let mut sources: Vec<(String, Source<'_>)> =
                    vec![("core".into(), Source::Open(self.store.db()))];
                for (name, path) in self.agent_db_files()? {
                    let id = name.trim_start_matches("agents/");
                    match open.iter().find(|(a, _)| a.to_string() == id) {
                        Some((_, s)) => sources.push((name, Source::Open(s.db()))),
                        None => sources.push((name, Source::File(path))),
                    }
                }
                backup::create_full_from(&dir, &sources, now)?.1
            }
            BackupKind::Incremental => {
                let base = last_full.ok_or_else(|| OrchError::Invalid("no full backup".into()))?;
                backup::create_incremental(&dir, &dir.join(&base.id), self.store.db(), now)?.1
            }
        };
        Ok(Some(m))
    }

    /// One maintenance tick: WAL gate on `core.db`, scheduled backup, retention.
    pub fn maintenance_tick(&self, now: DateTime<Utc>) -> Result<MaintenanceReport> {
        let wal_bytes = self.store.db().maintain()?;
        let made = self.backup(BackupRequest::Scheduled, now)?;
        let cfg = self.config().backup;
        let pruned = backup::prune(
            &self.backups_dir(),
            Retention {
                hourly: cfg.hourly_keep as usize,
                daily: cfg.daily_keep as usize,
                weekly: cfg.weekly_keep as usize,
            },
        )?;
        let keep_days = self.config().backup.run_log_keep_days;
        let logs_compacted = if keep_days == 0 {
            0
        } else {
            let cutoff = (now - chrono::Duration::days(i64::from(keep_days))).to_rfc3339();
            self.store
                .commit(None, "system", |w| {
                    agenium_store::core::runs::compact_logs(w.tx, &cutoff)
                        .map_err(crate::OrchError::from)
                })?
                .value
                .0
        };
        Ok(MaintenanceReport {
            wal_bytes,
            backup: made.map(|m| m.id),
            pruned,
            logs_compacted,
        })
    }
}

/// Restores a backup set of `from_root` into a fresh root (M0-11): databases (hash-verified) and `config.toml`.
/// Secrets are not part of backups; the vault file is copied only when present (its key stays in the keyring).
pub fn restore_into_new_root(
    from_root: &Path,
    backups_dir: &Path,
    set_id: &str,
    new_root: &Path,
) -> Result<Manifest> {
    let layout = agenium_config::RootLayout::new(new_root);
    if layout.core_db().exists() {
        return Err(OrchError::Conflict(format!(
            "{} already has a database",
            new_root.display()
        )));
    }
    layout.ensure()?;
    let m = backup::restore(backups_dir, set_id, &layout.db())?;
    for f in ["config.toml", "root-id", "vault.bin", "pricing.toml"] {
        let src = from_root.join(f);
        if src.is_file() {
            std::fs::copy(&src, new_root.join(f))
                .map_err(|e| OrchError::Invalid(format!("{}: {e}", src.display())))?;
        }
    }
    Ok(m)
}
