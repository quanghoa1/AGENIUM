//! A login lent to a run without its refresh token (ADR-137), for the CLIs whose login cannot be shared through a link
//! (Claude Code, Grok: they write it to a temporary file renamed over it, ADR-136). A refresh token is single-use: every
//! run that held a copy of it could use it, and the first refresh left the others with a used one ("log in again").
//! Claude Code does the same for its own temporary configuration folders: the copy has no refresh token, so it can
//! never refresh. A run lasts 45 minutes at most and an access token hours (Claude Code's 8, Grok's several), so a run
//! is lent the login without its refresh token whenever its access token outlives the run; the copy follows the user's
//! file (refreshed by the user's own CLI or by a run) within seconds, and the CLI reloads it. Only when the access token
//! runs out within the run does one run, one at a time per login, get the whole login and refresh it for everyone (its
//! refresh reaches the user's file as before, `sync_login`). A lent run whose token ran out with nobody refreshing it
//! takes that role after its login error (`promote`) and runs its turn again.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::login::{copy_login, original, replace, sync_login};

/// A CLI's login lent: the login without what refreshes it (nor what the run does not need), and when its access token
/// expires; `None` for a login of another shape (it is copied whole, as before).
pub type Lend = fn(&Value) -> Option<(Value, SystemTime)>;

/// Room left beyond the run's limit before the access token expires (the CLIs refresh a few minutes early).
const MARGIN: Duration = Duration::from_secs(15 * 60);

/// Per login file of the user, the login file of the one run that holds its refresh token.
static HOLDERS: Mutex<Vec<(PathBuf, PathBuf)>> = Mutex::new(Vec::new());

/// Gives a run its login: lent when its access token outlives the run (`run_for`) or another run holds the refresh
/// token, whole otherwise (this run may refresh it). Whether the run holds the refresh token.
pub fn lend_login(
    user_file: &Path,
    home_file: &Path,
    lend: Lend,
    run_for: Duration,
) -> std::io::Result<bool> {
    let raw = std::fs::read(user_file).map_err(|e| {
        std::io::Error::other(format!("not logged in ({}): {e}", user_file.display()))
    })?;
    let Some((lent, expires)) = serde_json::from_slice(&raw).ok().and_then(|v| lend(&v)) else {
        copy_login(user_file, home_file)?;
        return Ok(true);
    };
    if expires > SystemTime::now() + run_for + MARGIN || !take(user_file, home_file) {
        write_lent(home_file, &lent, &raw)?;
        return Ok(false);
    }
    copy_login(user_file, home_file)?;
    Ok(true)
}

/// Keeps a run's login as it should be, while the run goes on and at its end: a whole login is kept in step both ways
/// (`sync_login`); a lent one follows the user's file whenever it changes (a refresh elsewhere).
pub fn sync_lent(user_file: &Path, home_file: &Path, lend: Lend) -> std::io::Result<()> {
    if original(home_file).exists() {
        return sync_login(user_file, home_file);
    }
    let Ok(seen) = std::fs::read_to_string(mark(home_file)) else {
        return Ok(());
    };
    let raw = std::fs::read(user_file)?;
    if digest(&raw) == seen {
        return Ok(());
    }
    match serde_json::from_slice(&raw).ok().and_then(|v| lend(&v)) {
        Some((lent, _)) => write_lent(home_file, &lent, &raw),
        // Half a login (the user's CLI was writing it): the next sync takes it whole.
        None => Ok(()),
    }
}

/// After a lent run's login error: when the user's access token has run out (nobody refreshed it) and no other run
/// holds the refresh token, this run takes it and gets the whole login. Whether it did.
pub fn promote(user_file: &Path, home_file: &Path, lend: Lend) -> bool {
    if original(home_file).exists() {
        return false;
    }
    let fresh = std::fs::read(user_file)
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .and_then(|v: Value| lend(&v))
        .is_some_and(|(_, expires)| expires > SystemTime::now() + Duration::from_secs(5 * 60));
    if fresh || !take(user_file, home_file) {
        return false;
    }
    let _ = std::fs::remove_file(mark(home_file));
    copy_login(user_file, home_file).is_ok()
}

/// Ends a run's hold on a refresh token (its home is going).
pub fn release(home: &Path) {
    holders().retain(|(_, h)| !h.starts_with(home));
}

/// Ends the run's hold when dropped, however the run ends.
pub struct Released(pub PathBuf);

impl Drop for Released {
    fn drop(&mut self) {
        release(&self.0);
    }
}

fn holders() -> std::sync::MutexGuard<'static, Vec<(PathBuf, PathBuf)>> {
    HOLDERS.lock().unwrap_or_else(|p| p.into_inner())
}

/// Makes this run the holder of the refresh token of `user_file`, unless another run is.
fn take(user_file: &Path, home_file: &Path) -> bool {
    let mut h = holders();
    match h.iter().find(|(u, _)| u == user_file) {
        Some((_, other)) => other == home_file,
        None => {
            h.push((user_file.to_path_buf(), home_file.to_path_buf()));
            true
        }
    }
}

fn write_lent(home_file: &Path, lent: &Value, raw: &[u8]) -> std::io::Result<()> {
    replace(
        home_file,
        &serde_json::to_vec_pretty(lent).map_err(std::io::Error::other)?,
    )?;
    std::fs::write(mark(home_file), digest(raw))
}

/// The marker of a lent login: which state of the user's file it was made from.
fn mark(home_file: &Path) -> PathBuf {
    let mut name = home_file.file_name().unwrap_or_default().to_os_string();
    name.push(".agenium-lent");
    home_file.with_file_name(name)
}

fn digest(raw: &[u8]) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut h);
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::UNIX_EPOCH;

    /// A login of the test's shape: `access`, `refresh`, `exp` (seconds).
    fn lend(v: &Value) -> Option<(Value, SystemTime)> {
        let exp = v.get("exp")?.as_u64()?;
        Some((
            json!({"access": v["access"], "exp": exp}),
            UNIX_EPOCH + Duration::from_secs(exp),
        ))
    }

    fn login(access: &str, refresh: &str, in_secs: i64) -> Vec<u8> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        serde_json::to_vec(&json!({"access": access, "refresh": refresh, "exp": now + in_secs}))
            .unwrap()
    }

    fn read(p: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
    }

    /// ADR-137: a run whose access token outlives it is lent the login without the refresh token and follows the user's
    /// file; near the expiry one run at a time holds the whole login; a lent run takes it after its login error only when
    /// the token ran out and nobody holds it.
    #[test]
    fn a_login_is_lent_without_its_refresh_token_and_one_run_refreshes() {
        let tmp = tempfile::tempdir().unwrap();
        let user = tmp.path().join("user.json");
        let home = |n: &str| {
            let h = tmp.path().join(n);
            std::fs::create_dir_all(&h).unwrap();
            h.join("auth.json")
        };
        let (a, b, c) = (home("a"), home("b"), home("c"));
        let hour = Duration::from_secs(3600);
        std::fs::write(&user, login("x1", "r1", 8 * 3600)).unwrap();
        assert!(!lend_login(&user, &a, lend, hour).unwrap(), "lent");
        assert_eq!(read(&a)["access"], "x1");
        assert!(
            read(&a).get("refresh").is_none(),
            "no refresh token in the run"
        );
        // The user's own CLI refreshed: the lent copy follows within a sync.
        std::fs::write(&user, login("x2", "r2", 8 * 3600)).unwrap();
        sync_lent(&user, &a, lend).unwrap();
        assert_eq!(read(&a)["access"], "x2");
        // Near the expiry: one run holds the whole login, the next is lent.
        std::fs::write(&user, login("x3", "r3", 20 * 60)).unwrap();
        assert!(
            lend_login(&user, &b, lend, hour).unwrap(),
            "b holds the refresh token"
        );
        assert_eq!(read(&b)["refresh"], "r3");
        assert!(
            !lend_login(&user, &c, lend, hour).unwrap(),
            "c is lent: b refreshes"
        );
        // b's refresh reaches the user's file and c's copy.
        std::fs::write(&b, login("x4", "r4", 8 * 3600)).unwrap();
        sync_lent(&user, &b, lend).unwrap();
        sync_lent(&user, &c, lend).unwrap();
        assert_eq!(read(&user)["refresh"], "r4");
        assert_eq!(read(&c)["access"], "x4");
        assert!(read(&c).get("refresh").is_none());
        // A fresh token: a login error of c is not a reason to take the refresh token.
        assert!(!promote(&user, &c, lend));
        // The token ran out with b gone: c takes it.
        release(b.parent().unwrap());
        std::fs::write(&user, login("x5", "r5", -60)).unwrap();
        assert!(promote(&user, &c, lend));
        assert_eq!(read(&c)["refresh"], "r5");
        release(tmp.path());
    }
}
