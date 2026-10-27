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
    "__pycache__/",
    "*.py[cod]",
    ".pytest_cache/",
    ".mypy_cache/",
    ".ruff_cache/",
    ".venv/",
    "node_modules/",
    ".DS_Store",
    "Thumbs.db",
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

    /// `true` when `root` is the top level of a git repository.
    pub fn is_repo(&self) -> bool {
        self.git
            .run(&self.root, ["rev-parse", "--show-toplevel"])
            .map(|o| o.ok())
            .unwrap_or(false)
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

    /// The main repository's git directory (the one its worktrees share), absolute.
    fn common_dir(&self) -> Result<PathBuf, VcsError> {
        let out = self
            .git
            .check(&self.root, ["rev-parse", "--git-common-dir"])?;
        let dir = PathBuf::from(out.trim());
        let dir = if dir.is_absolute() {
            dir
        } else {
            self.root.join(dir)
        };
        Ok(std::path::absolute(&dir).unwrap_or(dir))
    }

    /// The git directory the main repository keeps for the linked worktree at `dir`: the `<common>/worktrees/<name>`
    /// whose `gitdir` file points back to `dir/.git`. Read from the main repository only, never from `dir` (its `.git`
    /// file is the agent's to rewrite). `None` when no worktree is registered at `dir`.
    fn worktree_git_dir(&self, dir: &Path) -> Result<Option<PathBuf>, VcsError> {
        let admin = self.common_dir()?.join("worktrees");
        let entries = match std::fs::read_dir(&admin) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io(&admin, e)),
        };
        for e in entries.flatten() {
            let git_dir = e.path();
            let Ok(back) = std::fs::read_to_string(git_dir.join("gitdir")) else {
                continue;
            };
            // Relative (relative to this directory) with `worktree.useRelativePaths`, git 2.48+.
            let back = git_dir.join(back.trim());
            if back.parent().is_some_and(|wt| Self::same_path(wt, dir)) {
                return Ok(Some(git_dir));
            }
        }
        Ok(None)
    }

    /// The git runner for `dir`: the repository's own working tree as it is; a registered linked worktree bound to the
    /// git directory the main repository keeps for it ([`Git::bound`]). Any other directory is refused rather than
    /// left to git's discovery, which trusts whatever `.git` it finds there.
    pub fn git_for(&self, dir: &Path) -> Result<Git, VcsError> {
        if Self::same_path(dir, &self.root) {
            return Ok(self.git.clone());
        }
        match self.worktree_git_dir(dir)? {
            Some(git_dir) => Ok(self.git.bound(git_dir)),
            None => Err(VcsError::Invalid(format!(
                "{} is not a worktree of the repository {}",
                dir.display(),
                self.root.display()
            ))),
        }
    }

    /// Adds `.agenium/` and the [`LOCAL_EXCLUDES`] missing from `.git/info/exclude` (local to this clone: the
    /// project's own `.gitignore` is not touched, and files already tracked stay tracked).
    fn exclude_local(&self) -> Result<(), VcsError> {
        let git_dir = self.common_dir()?;
        let info = git_dir.join("info");
        std::fs::create_dir_all(&info).map_err(|e| io(&info, e))?;
        let exclude = info.join("exclude");
        let cur = std::fs::read_to_string(&exclude).unwrap_or_default();
        let missing: Vec<&str> = std::iter::once(".agenium/")
            .chain(LOCAL_EXCLUDES.iter().copied())
            .filter(|p| !cur.lines().any(|l| l.trim() == *p))
            .collect();
        if !missing.is_empty() {
            let mut s = cur;
            if !s.is_empty() && !s.ends_with('\n') {
                s.push('\n');
            }
            for p in missing {
                s.push_str(p);
                s.push('\n');
            }
            std::fs::write(&exclude, s).map_err(|e| io(&exclude, e))?;
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

    /// Registered worktrees: (path, branch).
    pub fn worktrees(&self) -> Result<Vec<(PathBuf, Option<String>)>, VcsError> {
        let out = self
            .git
            .check(&self.root, ["worktree", "list", "--porcelain"])?;
        let mut v = Vec::new();
        let (mut path, mut branch) = (None::<PathBuf>, None::<String>);
        for line in out.lines().chain(std::iter::once("")) {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p));
            } else if let Some(b) = line.strip_prefix("branch ") {
                branch = Some(b.trim_start_matches("refs/heads/").to_owned());
            } else if line.is_empty() {
                if let Some(p) = path.take() {
                    v.push((p, branch.take()));
                }
            }
        }
        Ok(v)
    }

    fn same_path(a: &Path, b: &Path) -> bool {
        let norm = |p: &Path| {
            let s = std::fs::canonicalize(p)
                .unwrap_or_else(|_| p.to_path_buf())
                .to_string_lossy()
                .replace('\\', "/");
            let s = s.trim_start_matches("//?/").to_owned();
            if cfg!(windows) {
                s.to_lowercase()
            } else {
                s
            }
        };
        norm(a) == norm(b)
    }

    /// Creates (or finds) the worktree of ticket `key` branched from `base`. Idempotent, also after a crash: a
    /// registered worktree is matched by its path (one left in the middle of a rebase shows a detached HEAD, not its
    /// branch), its rebase is aborted, and its directory is never deleted. Treating it as unregistered once deleted the
    /// directory and then failed for good ("missing but already registered worktree").
    pub fn add_worktree(&self, key: &str, base: &str) -> Result<WorktreeInfo, VcsError> {
        // Projects created before a pattern joined the list get it here, before an agent works.
        self.exclude_local()?;
        let path = self.worktree_path(key);
        let branch = ticket_branch(key);
        let registered = self
            .worktrees()?
            .into_iter()
            .find(|(p, _)| Self::same_path(p, &path));
        let add = match registered {
            Some((_, on)) if path.exists() => {
                if on.as_deref() != Some(branch.as_str()) {
                    self.recover_worktree(&path, &branch)?;
                }
                false
            }
            Some(_) => {
                // Registered, but its directory is gone: drop the stale registration and add it again.
                self.git.check(&self.root, ["worktree", "prune"])?;
                true
            }
            None => {
                if path.exists() {
                    // Not a worktree (a crash between creating the directory and registering it, or a removed
                    // registration): recreate it.
                    self.git.check(&self.root, ["worktree", "prune"])?;
                    std::fs::remove_dir_all(&path).map_err(|e| io(&path, e))?;
                }
                true
            }
        };
        if add {
            std::fs::create_dir_all(self.worktrees_dir())
                .map_err(|e| io(&self.worktrees_dir(), e))?;
            let p = path.to_string_lossy().into_owned();
            if self.branch_exists(&branch)? {
                self.git
                    .check(&self.root, ["worktree", "add", "-q", &p, &branch])?;
            } else {
                self.git.check(
                    &self.root,
                    ["worktree", "add", "-q", "-b", &branch, &p, base],
                )?;
            }
        }
        let base_sha = self
            .git
            .check(&self.root, ["merge-base", &branch, base])?
            .trim()
            .to_owned();
        Ok(WorktreeInfo {
            path,
            branch,
            base_sha,
        })
    }

    /// A registered worktree that is not on its branch: a rebase interrupted by a crash (the merge queue's) is aborted,
    /// which puts the branch back where it was. Anything else is reported, never repaired by deleting the work.
    fn recover_worktree(&self, path: &Path, branch: &str) -> Result<(), VcsError> {
        let git_dir = self
            .worktree_git_dir(path)?
            .ok_or_else(|| VcsError::Invalid(format!("{} is not registered", path.display())))?;
        let g = self.git.bound(git_dir.clone());
        if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
            g.check(path, ["rebase", "--abort"])?;
        }
        let on = g.run(path, ["symbolic-ref", "-q", "--short", "HEAD"])?;
        if on.stdout.trim() != branch {
            return Err(VcsError::Invalid(format!(
                "the worktree {} is on {:?}, not on {branch}; resolve it by hand",
                path.display(),
                on.stdout.trim()
            )));
        }
        Ok(())
    }

    /// Brings the worktree of ticket `key` up to date with main when its branch has no commits of its own and no
    /// uncommitted changes (a dependent ticket assigned before its dependencies merged). Returns the new base.
    /// A branch with work of its own is left alone: the merge queue rebases it.
    pub fn catch_up(&self, key: &str) -> Result<Option<String>, VcsError> {
        let wt = self.worktree_path(key);
        let branch = ticket_branch(key);
        let (Some(head), Some(main)) = (self.head_sha(&branch)?, self.head_sha(&self.main)?) else {
            return Ok(None);
        };
        if head == main || !self.is_ancestor(&head, &self.main)? || !self.status(&wt)?.is_empty() {
            return Ok(None);
        }
        self.git_for(&wt)?
            .check(&wt, ["merge", "-q", "--ff-only", &self.main])?;
        Ok(Some(main))
    }

    /// Removes the worktree directory and registration (keeps the branch). Idempotent.
    pub fn remove_worktree(&self, key: &str) -> Result<(), VcsError> {
        let path = self.worktree_path(key);
        if path.exists() {
            let p = path.to_string_lossy().into_owned();
            let out = self
                .git
                .run(&self.root, ["worktree", "remove", "--force", &p])?;
            if !out.ok() && path.exists() {
                std::fs::remove_dir_all(&path).map_err(|e| io(&path, e))?;
            }
        }
        self.git.check(&self.root, ["worktree", "prune"])?;
        Ok(())
    }

    /// Deletes a ticket branch (abandoned tickets need `-D`, spike S4). Idempotent.
    pub fn delete_branch(&self, branch: &str) -> Result<(), VcsError> {
        if self.branch_exists(branch)? {
            self.git.check(&self.root, ["branch", "-D", branch])?;
        }
        Ok(())
    }

    /// Stages everything in the worktree, scans the staged changes for secrets and commits. Returns `None`
    /// when there was nothing to commit. Never commits a detected secret (decision 34).
    pub fn commit_all(
        &self,
        worktree: &Path,
        message: &str,
        author: (&str, &str),
        trailers: &[(&str, &str)],
    ) -> Result<Option<String>, VcsError> {
        let g = self.git_for(worktree)?;
        g.check(worktree, ["add", "-A"])?;
        let staged = g.run(worktree, ["diff", "--cached", "--quiet"])?;
        if staged.ok() {
            return Ok(None);
        }
        let findings = Self::staged_findings(&g, worktree)?;
        if !findings.is_empty() {
            // Leave the work in the tree but unstaged so nothing half-scanned is committed later by accident.
            g.check(worktree, ["reset", "-q"])?;
            return Err(VcsError::SecretsFound(findings));
        }
        let mut msg = message.trim().to_owned();
        if !trailers.is_empty() {
            msg.push_str("\n\n");
            for (k, v) in trailers {
                msg.push_str(&format!("{k}: {v}\n"));
            }
        }
        g.check_env(
            worktree,
            ["commit", "-q", "--no-verify", "-m", &msg],
            &[
                ("GIT_AUTHOR_NAME", author.0),
                ("GIT_AUTHOR_EMAIL", author.1),
            ],
        )?;
        Ok(Some(
            g.check(worktree, ["rev-parse", "HEAD"])?.trim().to_owned(),
        ))
    }

    /// Scans the added lines of the staged diff.
    pub fn scan_staged(&self, worktree: &Path) -> Result<Vec<StagedFinding>, VcsError> {
        Self::staged_findings(&self.git_for(worktree)?, worktree)
    }

    fn staged_findings(g: &Git, worktree: &Path) -> Result<Vec<StagedFinding>, VcsError> {
        let diff = g.check(
            worktree,
            [
                "diff",
                "--cached",
                "--no-color",
                "--unified=0",
                "--no-ext-diff",
            ],
        )?;
        Ok(scan_diff(&diff))
    }

    /// Unified diff of the branch since `base` (what the reviewer sees).
    pub fn diff(&self, worktree: &Path, base: &str) -> Result<String, VcsError> {
        self.git_for(worktree)?.check(
            worktree,
            [
                "diff",
                "--no-color",
                "--no-ext-diff",
                &format!("{base}..HEAD"),
            ],
        )
    }

    pub fn changed_files(&self, dir: &Path, from: &str, to: &str) -> Result<Vec<String>, VcsError> {
        let out = self.git_for(dir)?.check(
            dir,
            [
                "diff",
                "--name-only",
                "--no-renames",
                &format!("{from}..{to}"),
            ],
        )?;
        Ok(out
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// What a worktree holds: its HEAD and the tree of all its files (committed, changed, and untracked files that
    /// are not ignored), written with a scratch index in the worktree's git dir so the real index is untouched.
    pub fn worktree_state(&self, worktree: &Path) -> Result<WorktreeState, VcsError> {
        Self::state(&self.git_for(worktree)?, worktree)
    }

    fn state(g: &Git, worktree: &Path) -> Result<WorktreeState, VcsError> {
        let head = g.check(worktree, ["rev-parse", "HEAD"])?.trim().to_owned();
        let tree = Self::scratch_tree(g, worktree)?;
        Ok(WorktreeState { head, tree })
    }

    fn scratch_index(g: &Git, worktree: &Path) -> Result<PathBuf, VcsError> {
        let dir = g.check(worktree, ["rev-parse", "--absolute-git-dir"])?;
        Ok(PathBuf::from(dir.trim()).join("agenium-scratch.index"))
    }

    fn scratch_tree(g: &Git, worktree: &Path) -> Result<String, VcsError> {
        let index = Self::scratch_index(g, worktree)?;
        let idx = index.to_string_lossy().into_owned();
        let env = [("GIT_INDEX_FILE", idx.as_str())];
        let tree = (|| {
            g.check_env(worktree, ["read-tree", "HEAD"], &env)?;
            g.check_env(worktree, ["add", "-A"], &env)?;
            g.check_env(worktree, ["write-tree"], &env)
        })();
        let _ = std::fs::remove_file(&index);
        Ok(tree?.trim().to_owned())
    }

    /// Commits on the worktree's branch since `from` that Agenium did not make (no `Agenium-Intent` trailer) and that
    /// `main` does not hold (a catch-up with main is Agenium's).
    pub fn foreign_commits(&self, worktree: &Path, from: &str) -> Result<Vec<String>, VcsError> {
        let out = self.git_for(worktree)?.check(
            worktree,
            [
                "log",
                "--format=%H%x09%(trailers:key=Agenium-Intent,valueonly,separator=%x2C)",
                &format!("{from}..HEAD"),
                "--not",
                &self.main,
            ],
        )?;
        Ok(out
            .lines()
            .filter_map(|l| l.split_once('\t'))
            .filter(|(_, intent)| intent.trim().is_empty())
            .map(|(sha, _)| sha.to_owned())
            .collect())
    }

    /// Puts a worktree back to `to` (an earlier [`WorktreeState`]) after changes made outside Agenium: returns the
    /// binary patch of what is undone (to keep for the user), then resets the branch to `to.head` and the files to
    /// `to.tree` (files added since are removed); the index follows HEAD afterwards.
    pub fn restore_worktree(
        &self,
        worktree: &Path,
        to: &WorktreeState,
    ) -> Result<String, VcsError> {
        let g = self.git_for(worktree)?;
        let now = Self::state(&g, worktree)?;
        let patch = g.check(worktree, ["diff", "--binary", &to.tree, &now.tree])?;
        let added = g.check(
            worktree,
            [
                "diff",
                "--name-only",
                "--diff-filter=A",
                "-z",
                &to.tree,
                &now.tree,
            ],
        )?;
        if now.head != to.head {
            g.check(worktree, ["reset", "-q", "--soft", &to.head])?;
        }
        let index = Self::scratch_index(&g, worktree)?;
        let idx = index.to_string_lossy().into_owned();
        let env = [("GIT_INDEX_FILE", idx.as_str())];
        let written = (|| {
            g.check_env(worktree, ["read-tree", &to.tree], &env)?;
            g.check_env(worktree, ["checkout-index", "-a", "-f"], &env)
        })();
        let _ = std::fs::remove_file(&index);
        written?;
        for f in added.split('\0').filter(|f| !f.is_empty()) {
            let _ = std::fs::remove_file(worktree.join(f));
        }
        g.check(worktree, ["reset", "-q"])?;
        Ok(patch)
    }

    /// Uncommitted changes in a worktree (porcelain lines). Nested repositories count by the commit they are on, not by
    /// their own uncommitted changes: looking for those runs git inside them, with their config (the
    /// `--ignore-submodules` option, unlike the config, also outranks a `.gitmodules` the agent wrote).
    pub fn status(&self, worktree: &Path) -> Result<Vec<String>, VcsError> {
        let out = self.git_for(worktree)?.check(
            worktree,
            [
                "status",
                "--porcelain",
                "--untracked-files=all",
                "--ignore-submodules=dirty",
            ],
        )?;
        Ok(out
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// Rebases the worktree's branch onto `onto`. On conflict the rebase is aborted and the files are listed.
    pub fn rebase(&self, worktree: &Path, onto: &str) -> Result<RebaseOutcome, VcsError> {
        let g = self.git_for(worktree)?;
        let head = g.check(worktree, ["rev-parse", "HEAD"])?.trim().to_owned();
        let onto_sha = g.check(worktree, ["rev-parse", onto])?.trim().to_owned();
        if self.is_ancestor(&onto_sha, &head)? {
            return Ok(RebaseOutcome::Rebased { head });
        }
        let out = g.run(worktree, ["rebase", "-q", onto])?;
        if out.ok() {
            let head = g.check(worktree, ["rev-parse", "HEAD"])?.trim().to_owned();
            return Ok(RebaseOutcome::Rebased { head });
        }
        let files = g
            .run(
                worktree,
                [
                    "diff",
                    "--name-only",
                    "--diff-filter=U",
                    "--ignore-submodules=dirty",
                ],
            )?
            .stdout;
        let files: Vec<String> = files
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect();
        let _ = g.run(worktree, ["rebase", "--abort"])?;
        if files.is_empty() {
            return Err(VcsError::Git {
                args: format!("rebase {onto}"),
                status: out.status,
                stderr: out.stderr,
            });
        }
        Ok(RebaseOutcome::Conflict { files })
    }

    /// Merge-queue step for one ticket (the caller serialises calls per project): rebase on the latest main,
    /// run the pre-merge check in the worktree, fast-forward main. Idempotent: a branch already contained in
    /// main reports `AlreadyMerged`.
    pub fn merge_ticket(
        &self,
        key: &str,
        check: &dyn CheckRunner,
    ) -> Result<MergeOutcome, VcsError> {
        let wt = self.worktree_path(key);
        let branch = ticket_branch(key);
        let head = self
            .head_sha(&branch)?
            .ok_or_else(|| VcsError::Invalid(format!("branch {branch} does not exist")))?;
        if self.is_ancestor(&head, &self.main)? {
            return Ok(MergeOutcome::AlreadyMerged {
                sha: self.head_sha(&self.main)?.unwrap_or(head),
            });
        }
        // Full outputs that tools of a later run (the review) spilled into the worktree are not part of the work.
        let dirty: Vec<String> = self
            .status(&wt)?
            .into_iter()
            .filter(|l| !l.starts_with(&format!("?? {TOOL_OUTPUT_DIR}/")))
            .collect();
        if !dirty.is_empty() {
            return Err(VcsError::Invalid(format!(
                "worktree of {key} has uncommitted changes: {dirty:?}"
            )));
        }
        let main_before = self.head_sha(&self.main)?.unwrap_or_default();
        let head = match self.rebase(&wt, &self.main)? {
            RebaseOutcome::Rebased { head } => head,
            RebaseOutcome::Conflict { files } => return Ok(MergeOutcome::Conflict { files }),
        };
        let c = check.run(&wt);
        if !c.ok {
            return Ok(MergeOutcome::CheckFailed { log: c.log });
        }
        // The project's own working tree has main checked out: a fast-forward keeps it in sync. It fails if the
        // user edited main while the project was running (decision 19), which surfaces as an error. No hook runs:
        // a relative `core.hooksPath` (husky's `.husky`) would find the hooks the merged branch brings, written by
        // an agent, and run them here, on the host.
        self.git.check(
            &self.root,
            [
                "-c",
                git::NO_HOOKS,
                "-c",
                "core.fsmonitor=",
                "merge",
                "-q",
                "--ff-only",
                &branch,
            ],
        )?;
        let changed = self.changed_files(&self.root, &main_before, &head)?;
        Ok(MergeOutcome::Merged {
            sha: head,
            changed_files: changed,
        })
    }

    /// Tests a remote: read (`ls-remote`) and write (`push --dry-run`).
    pub fn test_remote(&self, url: &str, env: &[(&str, &str)]) -> Result<RemoteTest, VcsError> {
        let read = self
            .git
            .run_env(&self.root, ["ls-remote", "--heads", url], env)?;
        let write = if read.ok() {
            Some(self.git.run_env(
                &self.root,
                [
                    "push",
                    "--dry-run",
                    "--porcelain",
                    url,
                    &format!("{}:refs/heads/agenium-probe", self.main),
                ],
                env,
            )?)
        } else {
            None
        };
        Ok(RemoteTest {
            read_ok: read.ok(),
            read_detail: if read.ok() {
                format!("{} branches", read.stdout.lines().count())
            } else {
                read.stderr.trim().to_owned()
            },
            write_ok: write.as_ref().is_some_and(|w| w.ok()),
            write_detail: write
                .map(|w| {
                    if w.ok() {
                        "dry-run push accepted".into()
                    } else {
                        w.stderr.trim().to_owned()
                    }
                })
                .unwrap_or_default(),
        })
    }

    /// The URL of remote `name` (e.g. `origin`), or `None` when the repo has no such remote.
    pub fn remote_url(&self, name: &str) -> Option<String> {
        self.git
            .run(&self.root, ["remote", "get-url", name])
            .ok()
            .filter(|o| o.ok())
            .map(|o| o.stdout.trim().to_owned())
            .filter(|u| !u.is_empty())
    }

    /// Pushes a branch to a remote (used when a ticket closes and the project has a remote).
    pub fn push(&self, remote: &str, refspec: &str, env: &[(&str, &str)]) -> Result<(), VcsError> {
        self.git
            .check_env(&self.root, ["push", "--porcelain", remote, refspec], env)?;
        Ok(())
    }

    /// Commits of `branch` not in `main` whose message carries `Agenium-Intent: <intent>`.
    pub fn commit_with_intent(
        &self,
        refname: &str,
        intent: &str,
    ) -> Result<Option<String>, VcsError> {
        let out = self.git.run(
            &self.root,
            [
                "log",
                "--format=%H",
                &format!("--grep=Agenium-Intent: {intent}"),
                "--fixed-strings",
                refname,
            ],
        )?;
        Ok(out
            .ok()
            .then(|| out.stdout.lines().next().map(str::to_owned))
            .flatten())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteTest {
    pub read_ok: bool,
    pub read_detail: String,
    pub write_ok: bool,
    pub write_detail: String,
}

/// Scans the `+` lines of a unified diff, attributing findings to files and new-file line numbers.
pub fn scan_diff(diff: &str) -> Vec<StagedFinding> {
    let mut out = Vec::new();
    let mut file = String::new();
    let mut line_no = 0usize;
    for l in diff.lines() {
        if let Some(f) = l.strip_prefix("+++ ") {
            file = f.trim_start_matches("b/").to_owned();
        } else if let Some(h) = l.strip_prefix("@@ ") {
            // @@ -a,b +c,d @@
            line_no = h
                .split_whitespace()
                .find_map(|p| p.strip_prefix('+'))
                .and_then(|p| p.split(',').next())
                .and_then(|n| n.parse().ok())
                .unwrap_or(1);
        } else if let Some(added) = l.strip_prefix('+') {
            for Finding { rule, preview, .. } in agenium_secrets::scan_text(added) {
                out.push(StagedFinding {
                    file: file.clone(),
                    rule: rule.to_owned(),
                    line: line_no,
                    preview,
                });
            }
            line_no += 1;
        }
    }
    out
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
