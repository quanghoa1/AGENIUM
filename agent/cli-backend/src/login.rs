//! The user's CLI login in a run's per-run home: shared through a hard link where it can be (ADR-136), otherwise a copy
//! kept in step with the user's file (ADR-134); and the scratch folders of usage probes, which hold one too.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Copies the user's login file into the per-run home, with a pristine copy to compare against afterwards.
pub fn copy_login(user_file: &Path, home_file: &Path) -> std::io::Result<()> {
    let login = std::fs::read(user_file).map_err(|e| {
        std::io::Error::other(format!("not logged in ({}): {e}", user_file.display()))
    })?;
    std::fs::write(home_file, &login)?;
    std::fs::write(original(home_file), &login)
}

/// Gives the per-run home the user's own login file, not a copy: a hard link, so the run's CLI reads and writes the very
/// file the user's CLI and every other run do. A CLI that writes its login in place and reads it again before it
/// refreshes (Codex) then shares one refresh token across all its sessions as it does on the user's machine: a copy held
/// a token another run had used, and its refresh was refused ("refresh_token_reused"). Where no hard link can be made
/// (another volume), the login is copied and kept in step (`copy_login`, `sync_login`). Whether it linked.
pub fn link_login(user_file: &Path, home_file: &Path) -> std::io::Result<bool> {
    let _ = std::fs::remove_file(home_file);
    if std::fs::hard_link(user_file, home_file).is_ok() {
        return Ok(true);
    }
    copy_login(user_file, home_file).map(|()| false)
}

/// The pristine copy of a login file copied into a per-run home.
pub(crate) fn original(home_file: &Path) -> PathBuf {
    let mut name = home_file.file_name().unwrap_or_default().to_os_string();
    name.push(".agenium-original");
    home_file.with_file_name(name)
}

/// A fresh folder inside `scratch` for one usage or model-list probe of a CLI, deleted when dropped — on every path,
/// errors included — with the copy of the user's login it holds. Probes used to share `scratch/<legacy>` and leave the
/// login (OAuth refresh tokens) there in clear; that folder is removed too.
pub(crate) fn probe_dir(scratch: &Path, legacy: &str) -> Result<tempfile::TempDir, String> {
    let _ = std::fs::remove_dir_all(scratch.join(legacy));
    std::fs::create_dir_all(scratch).map_err(|e| e.to_string())?;
    tempfile::Builder::new()
        .prefix("probe-")
        .tempdir_in(scratch)
        .map_err(|e| e.to_string())
}

/// Keeps a run's copy of a login and the user's file the same, while the run goes on and after it (one CLI account in
/// several runs at once, ADR-56). A login the CLI refreshed in the run's home goes to the user's file at once: it went
/// at the run's end, while a run started meanwhile held the old refresh token, which is single-use, and failed. A login
/// refreshed elsewhere (another run, the user's own CLI) comes into the copy while the run has not refreshed its own.
/// Changed on both sides, the user's wins; one that does not parse (written halfway) moves nowhere.
pub fn sync_login(user_file: &Path, home_file: &Path) -> std::io::Result<()> {
    let (Ok(now), Ok(before)) = (std::fs::read(home_file), std::fs::read(original(home_file)))
    else {
        return Ok(());
    };
    let user = std::fs::read(user_file)?;
    // A login the CLI was stopped while writing would log the other side out.
    let json = |b: &[u8]| serde_json::from_slice::<Value>(b).is_ok();
    let whole = |b: &[u8]| !b.is_empty() && (!json(&before) || json(b));
    if now != before {
        if user != before {
            tracing::warn!(
                file = %user_file.display(),
                "login changed both inside and outside the run; keeping the user's"
            );
            // Settled: the next sync brings the user's into the copy.
            return replace(&original(home_file), &now);
        }
        if !whole(&now) {
            tracing::warn!(
                file = %user_file.display(),
                "the refreshed login is not whole; keeping the user's"
            );
            return Ok(());
        }
        replace(user_file, &now)?;
        return replace(&original(home_file), &now);
    }
    if user != before && whole(&user) {
        replace(home_file, &user)?;
        replace(&original(home_file), &user)?;
    }
    Ok(())
}

/// Replaces `p` with `content` at once: a temporary file of its own renamed over it (two runs writing the same
/// temporary name collided).
pub(crate) fn replace(p: &Path, content: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new_in(p.parent().unwrap_or_else(|| Path::new(".")))?;
    tmp.write_all(content)?;
    tmp.persist(p).map(|_| ()).map_err(|e| e.error)
}
