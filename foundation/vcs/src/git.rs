//! Thin, testable runner around the `git` executable.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::VcsError;

/// Identity Agenium uses as committer; the author is the agent.
pub const COMMITTER_NAME: &str = "Agenium";
pub const COMMITTER_EMAIL: &str = "agenium@localhost";

#[derive(Debug, Clone)]
pub struct Git {
    exe: PathBuf,
    /// A linked worktree's git directory, found through the main repository ([`Git::bound`]).
    git_dir: Option<PathBuf>,
}

impl Default for Git {
    fn default() -> Self {
        Self {
            exe: PathBuf::from("git"),
            git_dir: None,
        }
    }
}

/// A `core.hooksPath` that holds no hooks.
pub(crate) const NO_HOOKS: &str = if cfg!(windows) {
    "core.hooksPath=NUL"
} else {
    "core.hooksPath=/dev/null"
};

#[derive(Debug, Clone)]
pub struct GitOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl GitOutput {
    pub fn ok(&self) -> bool {
        self.status == 0
    }
}

impl Git {
    /// This runner for a linked worktree whose git directory is `git_dir` (taken from the main repository, see
    /// `Repo::git_for`). Host git must never take it from the worktree: the worktree's `.git` file is inside the
    /// sandbox's writable root, and the agent could point it at a repository whose config runs commands
    /// (`core.fsmonitor`, hooks, filter drivers) — run here, outside the sandbox. Every call passes `--git-dir` and
    /// `--work-tree`, and turns off what would still run the worktree's own content: fsmonitor, hooks (a relative
    /// `core.hooksPath` such as husky's `.husky` resolves inside the worktree) and the dirty check of nested
    /// repositories (it runs git, with their config, inside them). Command-line config reaches child gits too.
    pub fn bound(&self, git_dir: PathBuf) -> Git {
        Git {
            exe: self.exe.clone(),
            git_dir: Some(git_dir),
        }
    }

    fn command(&self, dir: &Path) -> Command {
        let mut c = Command::new(&self.exe);
        c.arg("-C").arg(dir);
        if let Some(git_dir) = &self.git_dir {
            let mut gd = OsString::from("--git-dir=");
            gd.push(git_dir);
            let mut wt = OsString::from("--work-tree=");
            wt.push(std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf()));
            c.arg(gd).arg(wt);
            // Empty `core.fsmonitor` is "off" for every git since MIN_GIT (2.30 reads `false` as a hook path).
            c.args([
                "-c",
                "core.fsmonitor=",
                "-c",
                NO_HOOKS,
                "-c",
                "diff.ignoreSubmodules=dirty",
            ]);
        }
        // Deterministic, non-interactive behaviour: no pager, no prompts, no editor, C locale for parsing, long
        // paths on Windows, no CRLF rewriting of agent output.
        c.args([
            "-c",
            "core.pager=cat",
            "-c",
            "core.longpaths=true",
            "-c",
            "core.autocrlf=false",
            "-c",
            "advice.detachedHead=false",
        ]);
        c.env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_EDITOR", ":")
            .env("GIT_SEQUENCE_EDITOR", ":")
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("GIT_COMMITTER_NAME", COMMITTER_NAME)
            .env("GIT_COMMITTER_EMAIL", COMMITTER_EMAIL)
            .stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            c.creation_flags(CREATE_NO_WINDOW);
        }
        c
    }

    /// Runs git in `dir` and returns its output whatever the exit status.
    pub fn run<I, S>(&self, dir: &Path, args: I) -> Result<GitOutput, VcsError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_env(dir, args, &[])
    }

    pub fn run_env<I, S>(
        &self,
        dir: &Path,
        args: I,
        env: &[(&str, &str)],
    ) -> Result<GitOutput, VcsError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut c = self.command(dir);
        c.args(args);
        for (k, v) in env {
            c.env(k, v);
        }
        let out: Output = c
            .output()
            .map_err(|e| VcsError::GitMissing(e.to_string()))?;
        Ok(GitOutput {
            status: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    /// Runs git and fails on a non-zero exit with the command and stderr.
    pub fn check<I, S>(&self, dir: &Path, args: I) -> Result<String, VcsError>
    where
        I: IntoIterator<Item = S> + Clone,
        S: AsRef<OsStr>,
    {
        self.check_env(dir, args, &[])
    }

    pub fn check_env<I, S>(
        &self,
        dir: &Path,
        args: I,
        env: &[(&str, &str)],
    ) -> Result<String, VcsError>
    where
        I: IntoIterator<Item = S> + Clone,
        S: AsRef<OsStr>,
    {
        let shown: Vec<String> = args
            .clone()
            .into_iter()
            .map(|a| a.as_ref().to_string_lossy().into_owned())
            .collect();
        let out = self.run_env(dir, args, env)?;
        if out.ok() {
            Ok(out.stdout)
        } else {
            Err(VcsError::Git {
                args: shown.join(" "),
                status: out.status,
                stderr: out.stderr.trim().to_owned(),
            })
        }
    }

    /// `(major, minor)` of the installed git.
    pub fn version(&self) -> Result<(u32, u32), VcsError> {
        let out = self.check(Path::new("."), ["--version"])?;
        let v = out.trim().strip_prefix("git version ").unwrap_or_default();
        let mut it = v.split('.');
        match (
            it.next().and_then(|x| x.parse().ok()),
            it.next().and_then(|x| x.parse().ok()),
        ) {
            (Some(a), Some(b)) => Ok((a, b)),
            _ => Err(VcsError::GitMissing(format!("cannot parse {out:?}"))),
        }
    }
}
