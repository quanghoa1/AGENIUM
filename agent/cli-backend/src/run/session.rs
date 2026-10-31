//! The CLI's own session of a ticket, kept between runs (ADR-134). A ticket often takes several runs of its agent (a
//! question answered, a review reopened, a quota used up). Its per-run home, where the CLI keeps its session, went with
//! each run, so the next run started a new session shown the conversation so far as text. Now the home is kept (its
//! login copy never) per agent, ticket and CLI, brought back into the next run's home, and the CLI resumes its own
//! session there with only what is new; another CLI, a fresh start or a resume the CLI refuses goes back to the text.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::driver::Driver;

/// Where a ticket's CLI session is kept between runs, and how much of the work order it already holds.
pub struct SessionStore {
    /// One folder per agent and ticket.
    pub dir: PathBuf,
    /// Messages of the work order the kept session already holds (the earlier runs' conversation); 0 for a fresh start,
    /// which resumes nothing.
    pub held: usize,
}

/// A kept session older than this belongs to a ticket long ended.
const STALE: Duration = Duration::from_secs(14 * 24 * 3600);

/// What a kept session is.
#[derive(serde::Serialize, serde::Deserialize)]
struct Kept {
    cli: String,
    id: String,
    /// Messages of the ticket's conversation it holds: the next run's earlier messages must be these, or another run
    /// (an API model, another CLI) went on in between and the session misses its work.
    total: usize,
}

const KEPT: &str = "agenium-session.json";
const HOME: &str = "home";

impl SessionStore {
    /// Brings a kept session of `cli` back into `home` (before the run's own files are written there); its id, or
    /// `None` when there is nothing to resume.
    pub fn restore(&self, home: &Path, driver: &dyn Driver, cli: &str) -> Option<String> {
        if self.held == 0 || !driver.resumes_across_runs() {
            return None;
        }
        let kept: Kept = serde_json::from_slice(&std::fs::read(self.dir.join(KEPT)).ok()?).ok()?;
        if kept.cli != cli || kept.id.is_empty() || kept.total != self.held {
            return None;
        }
        copy_tree(&self.dir.join(HOME), home, &[]).ok()?;
        Some(kept.id)
    }

    /// Keeps the run's home (without the login copy), the CLI's session id and how many messages of the ticket's
    /// conversation it holds (`total`) for the ticket's next run; a run with no session, or a CLI that cannot resume
    /// one, keeps nothing (and ends what was kept).
    pub fn keep(
        &self,
        home: &Path,
        driver: &dyn Driver,
        cli: &str,
        id: Option<&str>,
        total: usize,
    ) {
        let _ = std::fs::remove_dir_all(&self.dir);
        let (Some(id), true) = (id, driver.resumes_across_runs()) else {
            return;
        };
        // The login and every file Agenium keeps beside it (its pristine copy, last whole state, lent marker) stay out.
        let login = driver.login_file();
        let mut skip: Vec<PathBuf> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(home) {
            skip.extend(
                rd.flatten()
                    .filter(|e| {
                        let name = e.file_name().to_string_lossy().into_owned();
                        driver.not_kept(&name)
                            || login.is_some_and(|l| name.starts_with(l))
                            || crate::driver::home_config_name(&name)
                    })
                    .map(|e| e.path()),
            );
        }
        let kept = Kept {
            cli: cli.to_owned(),
            id: id.to_owned(),
            total,
        };
        let r = copy_tree(home, &self.dir.join(HOME), &skip).and_then(|()| {
            std::fs::write(
                self.dir.join(KEPT),
                serde_json::to_vec(&kept).unwrap_or_default(),
            )
        });
        if let Err(e) = r {
            tracing::warn!(error = %e, "the CLI's session could not be kept");
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Removes kept sessions under `parent` that no run has touched for two weeks.
    pub fn sweep(parent: &Path) {
        let Ok(rd) = std::fs::read_dir(parent) else {
            return;
        };
        for e in rd.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > STALE);
            if old {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

/// Copies `src` into `dst` (made if missing), leaving out `skip`.
fn copy_tree(src: &Path, dst: &Path, skip: &[PathBuf]) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)?.flatten() {
        let (from, to) = (e.path(), dst.join(e.file_name()));
        if skip.contains(&from) {
            continue;
        }
        let kind = e.file_type()?;
        if kind.is_dir() {
            copy_tree(&from, &to, skip)?;
        } else if kind.is_file() {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::original;

    /// The home is kept without its login copy, and resumed only by the same CLI when the next run's earlier messages
    /// are the ones the session holds (another run in between, another CLI or a fresh start resume nothing).
    #[test]
    fn a_session_is_kept_without_the_login_and_resumed_only_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let (home, next, kept) = (
            tmp.path().join("home"),
            tmp.path().join("next"),
            tmp.path().join("kept"),
        );
        std::fs::create_dir_all(home.join("projects/wt")).unwrap();
        std::fs::write(home.join("projects/wt/s1.jsonl"), "turns").unwrap();
        std::fs::write(home.join(".credentials.json"), "secret").unwrap();
        std::fs::write(original(&home.join(".credentials.json")), "secret").unwrap();
        let claude = &crate::claude::Claude;
        let store = |held| SessionStore {
            dir: kept.clone(),
            held,
        };
        store(0).keep(&home, claude, "claude", Some("s1"), 5);
        assert!(!kept.join("home/.credentials.json").exists());
        assert!(!original(&kept.join("home/.credentials.json")).exists());
        assert_eq!(
            store(5).restore(&next, claude, "claude").as_deref(),
            Some("s1")
        );
        assert_eq!(
            std::fs::read_to_string(next.join("projects/wt/s1.jsonl")).unwrap(),
            "turns"
        );
        assert!(
            store(6).restore(&next, claude, "claude").is_none(),
            "a run went on in between"
        );
        assert!(
            store(5).restore(&next, claude, "codex").is_none(),
            "another CLI"
        );
        assert!(
            store(0).restore(&next, claude, "claude").is_none(),
            "a fresh start"
        );
        store(0).keep(&home, claude, "claude", None, 5);
        assert!(!kept.exists(), "a run with no session keeps nothing");
        // Codex's databases hold the old home's path: its sessions are kept without them.
        std::fs::write(home.join("state_5.sqlite"), "paths").unwrap();
        std::fs::write(home.join("state_5.sqlite-wal"), "paths").unwrap();
        store(0).keep(&home, &crate::codex::Codex, "codex", Some("t1"), 5);
        assert!(kept.join("home/projects/wt/s1.jsonl").exists());
        assert!(
            !kept.join("home/state_5.sqlite").exists()
                && !kept.join("home/state_5.sqlite-wal").exists()
        );
    }
}
