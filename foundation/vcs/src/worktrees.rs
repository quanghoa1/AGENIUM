//! Ticket worktrees: where they are, added, recovered, caught up, removed, and their state.

use super::*;

impl Repo {
    /// The main repository's git directory (the one its worktrees share), absolute.
    pub(super) fn common_dir(&self) -> Result<PathBuf, VcsError> {
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
    pub(super) fn worktree_git_dir(&self, dir: &Path) -> Result<Option<PathBuf>, VcsError> {
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
    pub(super) fn exclude_local(&self) -> Result<(), VcsError> {
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

    pub(super) fn same_path(a: &Path, b: &Path) -> bool {
        let norm = |p: &Path| {
            let s = resolved(p).to_string_lossy().replace('\\', "/");
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
    pub(super) fn recover_worktree(&self, path: &Path, branch: &str) -> Result<(), VcsError> {
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
                // Windows: a process that just ended (the agent's CLI, a server it started, an antivirus scan) can
                // hold a file for a moment; a few short retries spare a journaled retry and its warning.
                let mut wait = std::time::Duration::from_millis(200);
                let mut tries = 0;
                while let Err(e) = std::fs::remove_dir_all(&path) {
                    tries += 1;
                    if tries >= 5 || !path.exists() {
                        if path.exists() {
                            return Err(io(&path, e));
                        }
                        break;
                    }
                    std::thread::sleep(wait);
                    wait *= 2;
                }
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

    /// What a worktree holds: its HEAD and the tree of all its files (committed, changed, and untracked files that
    /// are not ignored), written with a scratch index in the worktree's git dir so the real index is untouched.
    pub fn worktree_state(&self, worktree: &Path) -> Result<WorktreeState, VcsError> {
        Self::state(&self.git_for(worktree)?, worktree)
    }

    pub(super) fn state(g: &Git, worktree: &Path) -> Result<WorktreeState, VcsError> {
        let head = g.check(worktree, ["rev-parse", "HEAD"])?.trim().to_owned();
        let tree = Self::scratch_tree(g, worktree)?;
        Ok(WorktreeState { head, tree })
    }

    pub(super) fn scratch_index(g: &Git, worktree: &Path) -> Result<PathBuf, VcsError> {
        let dir = g.check(worktree, ["rev-parse", "--absolute-git-dir"])?;
        Ok(PathBuf::from(dir.trim()).join("agenium-scratch.index"))
    }

    pub(super) fn scratch_tree(g: &Git, worktree: &Path) -> Result<String, VcsError> {
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
}

/// `p` with its links resolved as far as it exists, the rest as written: a worktree whose directory is gone still
/// matches git's record of it, which has the links resolved (`/var` is `/private/var` on macOS).
fn resolved(p: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = p;
    loop {
        if let Ok(real) = std::fs::canonicalize(at) {
            return rest.iter().rev().fold(real, |acc, name| acc.join(name));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                at = parent;
            }
            _ => return p.to_path_buf(),
        }
    }
}
