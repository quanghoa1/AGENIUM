//! Rebasing a ticket branch and merging it into main.

use super::*;

impl Repo {
    /// Rebases the worktree's branch onto `onto`. On conflict the rebase is aborted and the files are listed.
    pub fn rebase(&self, worktree: &Path, onto: &str) -> Result<RebaseOutcome, VcsError> {
        self.rebase_with(worktree, onto, false)
    }

    /// [`Repo::rebase`]; `keep_branch`: hunks that still conflict keep the branch's version (`-X theirs`, which in a
    /// rebase is the commits being replayed), for a worker that already brought `onto`'s change into its files.
    pub fn rebase_with(
        &self,
        worktree: &Path,
        onto: &str,
        keep_branch: bool,
    ) -> Result<RebaseOutcome, VcsError> {
        let g = self.git_for(worktree)?;
        let head = g.check(worktree, ["rev-parse", "HEAD"])?.trim().to_owned();
        let onto_sha = g.check(worktree, ["rev-parse", onto])?.trim().to_owned();
        if self.is_ancestor(&onto_sha, &head)? {
            return Ok(RebaseOutcome::Rebased { head });
        }
        let out = if keep_branch {
            g.run(worktree, ["rebase", "-q", "-X", "theirs", onto])?
        } else {
            g.run(worktree, ["rebase", "-q", onto])?
        };
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
        match self.prepare_merge(key, check)? {
            Prepared::Ready { head, main_before } => self.fast_forward(&head, &main_before),
            Prepared::Done(o) => Ok(o),
        }
    }

    /// The slow part of [`Repo::merge_ticket`]: rebase on the latest main and the pre-merge check. A ticket can be
    /// reopened or cancelled while it runs (minutes for a test suite), so the caller decides afterwards whether the
    /// checked head still goes into main, and then calls [`Repo::fast_forward`].
    pub fn prepare_merge(&self, key: &str, check: &dyn CheckRunner) -> Result<Prepared, VcsError> {
        let wt = self.worktree_path(key);
        let branch = ticket_branch(key);
        let head = self
            .head_sha(&branch)?
            .ok_or_else(|| VcsError::Invalid(format!("branch {branch} does not exist")))?;
        if self.is_ancestor(&head, &self.main)? {
            return Ok(Prepared::Done(MergeOutcome::AlreadyMerged {
                sha: self.head_sha(&self.main)?.unwrap_or(head),
            }));
        }
        // A rebase of an earlier attempt the process died in is aborted first, which puts the branch back: read as it
        // was, its partly rebased HEAD passed for rebased, was checked and could be fast-forwarded into main (audit
        // M11, 2026-10-31). A worktree off its branch is an error, never merged.
        self.recover_worktree(&wt, &branch)?;
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
        let head = match self.with_tool_output_aside(&wt, key, || self.rebase(&wt, &self.main))? {
            RebaseOutcome::Rebased { head } => head,
            RebaseOutcome::Conflict { files } => {
                return Ok(Prepared::Done(MergeOutcome::Conflict { files }))
            }
        };
        let c = check.run(&wt);
        if !c.ok {
            return Ok(Prepared::Done(MergeOutcome::CheckFailed { log: c.log }));
        }
        Ok(Prepared::Ready { head, main_before })
    }

    /// Fast-forwards main to `head`, the commit [`Repo::prepare_merge`] rebased and checked (not the branch's tip,
    /// which may have moved since). The project's own working tree has main checked out: a fast-forward keeps it in
    /// sync. It fails if the user edited main while the project was running (decision 19), which surfaces as an
    /// error. No hook runs: a relative `core.hooksPath` (husky's `.husky`) would find the hooks the merged branch
    /// brings, written by an agent, and run them here, on the host.
    pub fn fast_forward(&self, head: &str, main_before: &str) -> Result<MergeOutcome, VcsError> {
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
                head,
            ],
        )?;
        let changed = self.changed_files(&self.root, main_before, head)?;
        Ok(MergeOutcome::Merged {
            sha: head.to_owned(),
            changed_files: changed,
        })
    }

    /// Runs `f` with the worktree's tool outputs moved into the git directory: a commit of the branch made before
    /// they were kept out may hold one, and replaying it on a rebase fails over the untracked copy on disk. They are
    /// put back afterwards when the place is free (else they stay under `<git dir>/agenium-tool-output/<key>`).
    pub(super) fn with_tool_output_aside<T>(
        &self,
        wt: &Path,
        key: &str,
        f: impl FnOnce() -> Result<T, VcsError>,
    ) -> Result<T, VcsError> {
        let dir = wt.join(TOOL_OUTPUT_DIR);
        if !dir.is_dir() {
            return f();
        }
        let aside = self.common_dir()?.join("agenium-tool-output").join(key);
        let _ = std::fs::remove_dir_all(&aside);
        if let Some(parent) = aside.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        std::fs::rename(&dir, &aside).map_err(|e| io(&dir, e))?;
        let r = f();
        if !dir.exists() {
            // Best effort: on failure they stay in the git directory, which is no loss to the work. The rebase may
            // have removed the emptied `artifacts/`.
            if let Some(parent) = dir.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::rename(&aside, &dir);
        }
        r
    }
}
