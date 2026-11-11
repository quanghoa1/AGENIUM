//! Git for Agenium (decision 11, ADR-44): every project is a git repository; every ticket works in its own
//! worktree `<project>/.agenium/worktrees/<KEY>` on branch `agenium/<KEY>`; results reach `main` only through
//! the sequential merge queue (rebase on the latest main -> pre-merge check -> fast-forward merge).
//!
//! Operations are idempotent per (ticket, sha) so the orchestrator can re-run them after a crash: it journals an
//! intent event first, then calls these, then journals completion (decision 22). Commits made by Agenium carry
//! the trailers `Ticket: <KEY>` and `Agenium-Intent: <id>` so reconcile can recognise them in git history.

pub mod git;

use std::path::{Path, PathBuf};

use agenium_secrets::Finding;
use serde::{Deserialize, Serialize};

pub use git::Git;

mod worktrees;

pub use commits::scan_diff;
pub use remote::RemoteTest;
mod commits;
mod merge;
mod remote;

pub const MIN_GIT: (u32, u32) = (2, 30);

/// Worktree-relative folder where tools write full outputs (`artifacts/tool-output/<run>`, agenium-tools).
pub const TOOL_OUTPUT_DIR: &str = "artifacts/tool-output";

#[derive(Debug, thiserror::Error)]
pub enum VcsError {
    #[error("git is not available: {0}")]
    GitMissing(String),
    #[error("git {args} failed ({status}): {stderr}")]
    Git {
        args: String,
        status: i32,
        stderr: String,
    },
    #[error("secret detected in staged changes; commit blocked: {0:?}")]
    SecretsFound(Vec<StagedFinding>),
    #[error("{0}")]
    Invalid(String),
    #[error("io {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedFinding {
    pub file: String,
    pub rule: String,
    pub line: usize,
    pub preview: String,
}

/// Branch name of a ticket.
pub fn ticket_branch(key: &str) -> String {
    format!("agenium/{key}")
}

/// Files tools generate while an agent builds or tests (caches, installed packages, OS litter): never part of a
/// ticket's change. Kept out of every commit and of the write-scope check through `.git/info/exclude`, so a
/// `python -m unittest` does not put `__pycache__` in `main` or fail a ticket for a file nobody wrote.
pub const LOCAL_EXCLUDES: &[&str] = &[
    "artifacts/tool-output/",
    "__pycache__/",
    "*.py[cod]",
    ".pytest_cache/",
    ".mypy_cache/",
    ".ruff_cache/",
    ".venv/",
    "node_modules/",
    ".DS_Store",
    "Thumbs.db",
    // The file a Devin run writes to turn the user's own Devin MCP servers off (ADR-156).
    ".devin/mcp_config.local.json",
];

/// A project repository (its own working tree has `main` checked out).
#[derive(Debug, Clone)]
pub struct Repo {
    pub root: PathBuf,
    pub main: String,
    git: Git,
}

/// A worktree's content at one moment (decision 19: what Agenium left it with after a run).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeState {
    pub head: String,
    /// Tree of every file in the worktree, uncommitted and untracked (not ignored) included.
    pub tree: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: String,
    pub base_sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RebaseOutcome {
    /// Branch now sits on top of `onto` (or already did).
    Rebased { head: String },
    /// Rebase aborted; the branch is unchanged.
    Conflict { files: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckOutcome {
    pub ok: bool,
    pub log: String,
}

/// Pre-merge check of the team template (tests, schema validation, citation check, ...). The orchestrator
/// decides how it runs (sandbox / approval policy).
pub trait CheckRunner: Send + Sync {
    fn run(&self, worktree: &Path) -> CheckOutcome;
}

/// A check that always passes (templates without a pre-merge check).
pub struct NoCheck;
impl CheckRunner for NoCheck {
    fn run(&self, _: &Path) -> CheckOutcome {
        CheckOutcome {
            ok: true,
            log: String::new(),
        }
    }
}

/// What [`Repo::prepare_merge`] leaves: a rebased and checked head ready for [`Repo::fast_forward`], or the outcome
/// already (merged before, a conflict, a failed check).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prepared {
    Ready { head: String, main_before: String },
    Done(MergeOutcome),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeOutcome {
    Merged {
        sha: String,
        changed_files: Vec<String>,
    },
    /// Already contained in main (idempotent replay).
    AlreadyMerged {
        sha: String,
    },
    Conflict {
        files: Vec<String>,
    },
    CheckFailed {
        log: String,
    },
}

/// Whether two paths name one folder (git prints `D:/x` where Windows gives `D:\x`, links resolved).
pub fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn io(path: &Path, source: std::io::Error) -> VcsError {
    VcsError::Io {
        path: path.to_path_buf(),
        source,
    }
}

impl Repo {
    pub fn open(root: &Path, main: &str) -> Self {
        Self {
            root: root.to_path_buf(),
            main: main.to_owned(),
            git: Git::default(),
        }
    }

    /// The git runner for the repository root. Never run it in a worktree: there git would trust the worktree's own
    /// `.git` file; [`Repo::git_for`] gives the runner for a worktree.
    pub fn git(&self) -> &Git {
        &self.git
    }

    pub fn worktrees_dir(&self) -> PathBuf {
        self.root.join(".agenium").join("worktrees")
    }

    pub fn worktree_path(&self, key: &str) -> PathBuf {
        self.worktrees_dir().join(key)
    }

    /// `true` when `root` is the top level of a git repository (a folder inside another repository is not one).
    pub fn is_repo(&self) -> bool {
        self.toplevel().is_some_and(|t| same_dir(&t, &self.root))
    }

    /// The top level of the repository `root` is in (itself or a parent), `None` outside any.
    pub fn toplevel(&self) -> Option<PathBuf> {
        let o = self
            .git
            .run(&self.root, ["rev-parse", "--show-toplevel"])
            .ok()?;
        o.ok()
            .then(|| PathBuf::from(o.stdout.trim()))
            .filter(|p| !p.as_os_str().is_empty())
    }

    /// Makes `root` a repository with branch `main` and at least one commit (git init when needed), adds
    /// `.agenium/` and the [`LOCAL_EXCLUDES`] to `.git/info/exclude`. Idempotent. A repository Agenium creates gets
    /// `core.autocrlf=false` so agent output is stored byte for byte.
    pub fn init_local(&self) -> Result<(), VcsError> {
        std::fs::create_dir_all(&self.root).map_err(|e| io(&self.root, e))?;
        if !self.is_repo() {
            self.git
                .check(&self.root, ["init", "-q", "-b", &self.main])?;
            self.git
                .check(&self.root, ["config", "core.autocrlf", "false"])?;
        }
        self.exclude_local()?;
        if self.head_sha(&self.main)?.is_none() {
            let current = self
                .git
                .run(&self.root, ["symbolic-ref", "--short", "HEAD"])?;
            if current.ok() && current.stdout.trim() != self.main {
                return Err(VcsError::Invalid(format!(
                    "repository HEAD is {:?} but the project main branch is {:?}",
                    current.stdout.trim(),
                    self.main
                )));
            }
            self.git.check_env(
                &self.root,
                [
                    "commit",
                    "-q",
                    "--allow-empty",
                    "-m",
                    "Initial commit (Agenium)",
                ],
                &[
                    ("GIT_AUTHOR_NAME", git::COMMITTER_NAME),
                    ("GIT_AUTHOR_EMAIL", git::COMMITTER_EMAIL),
                ],
            )?;
        }
        Ok(())
    }

    /// Sha of a ref, `None` when it does not exist.
    pub fn head_sha(&self, refname: &str) -> Result<Option<String>, VcsError> {
        let out = self.git.run(
            &self.root,
            [
                "rev-parse",
                "--verify",
                "-q",
                &format!("{refname}^{{commit}}"),
            ],
        )?;
        Ok(out.ok().then(|| out.stdout.trim().to_owned()))
    }

    pub fn branch_exists(&self, branch: &str) -> Result<bool, VcsError> {
        Ok(self
            .git
            .run(
                &self.root,
                [
                    "show-ref",
                    "--verify",
                    "-q",
                    &format!("refs/heads/{branch}"),
                ],
            )?
            .ok())
    }

    /// `true` when `sha` is contained in `refname`.
    pub fn is_ancestor(&self, sha: &str, refname: &str) -> Result<bool, VcsError> {
        let out = self
            .git
            .run(&self.root, ["merge-base", "--is-ancestor", sha, refname])?;
        match out.status {
            0 => Ok(true),
            1 => Ok(false),
            _ => Err(VcsError::Git {
                args: "merge-base --is-ancestor".into(),
                status: out.status,
                stderr: out.stderr,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_diff_attributes_lines() {
        let d = "diff --git a/x b/x\n+++ b/src/cfg.rs\n@@ -0,0 +10,2 @@\n+let a = 1;\n+let k = \"ghp_abcdefghijklmnopqrstuvwxyz0123456789\";\n";
        let f = scan_diff(d);
        assert_eq!(f.len(), 1);
        assert_eq!(
            (f[0].file.as_str(), f[0].line, f[0].rule.as_str()),
            ("src/cfg.rs", 11, "github-token")
        );
    }
}
