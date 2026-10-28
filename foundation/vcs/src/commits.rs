//! Commits of a worktree: what changed, the secret scan of what is staged, commits Agenium did not make.

use super::*;

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

impl Repo {
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
        // Tools' full outputs are never part of a change; one committed before they were excluded leaves the branch
        // here (the file stays on disk for the agent).
        g.check(
            worktree,
            [
                "rm",
                "-r",
                "-q",
                "--cached",
                "--ignore-unmatch",
                TOOL_OUTPUT_DIR,
            ],
        )?;
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

    pub(super) fn staged_findings(
        g: &Git,
        worktree: &Path,
    ) -> Result<Vec<StagedFinding>, VcsError> {
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
