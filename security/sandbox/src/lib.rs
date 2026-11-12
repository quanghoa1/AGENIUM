//! Command execution for agent-generated commands (decision 9, ADR-39/41/42/53).
//!
//! The sandbox applies only to commands an agent runs (shell, MCP stdio), never to the harness. This crate:
//! * detects which isolation backend works on this machine and **fails closed**: YOLO (no approval) is only
//!   allowed with a working backend. Windows: Microsoft MXC process containers through `mxc-sdk` (ADR-53,
//!   spike S2), when the native API is usable; Linux (bubblewrap / Landlock) and macOS (Seatbelt) are M2;
//! * runs commands with a cleared environment (no API keys), a timeout and a process-tree kill; with a backend
//!   the command runs inside the container whatever the approval policy.
//!
//! Without a backend every agent command runs through [`Executor::run`] only after the user approved it (M1-7).

pub mod codex_derived;
#[cfg(windows)]
pub mod mxc;
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
pub mod seatbelt;
pub mod service;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod unix;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

mod blocked;
mod gitconfig;
mod output;
mod policy;
mod shell;
mod tree;

pub use blocked::blocked_by_sandbox;
use gitconfig::agent_gitconfig;
use output::read_all;
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
pub(crate) use output::reader;
pub use output::Live;
pub use policy::*;
pub use shell::*;
pub use tree::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backend {
    Bubblewrap,
    LandlockSeccomp,
    Seatbelt,
    /// Windows: Microsoft MXC process container (`mxc-sdk`, ADR-53).
    WindowsMxc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Availability {
    pub backend: Option<Backend>,
    /// Why no backend is available (shown to the user, decision 24).
    pub reason: String,
}

static OFF: AtomicBool = AtomicBool::new(false);

/// The user turned the sandbox off on this computer (`[sandbox] off`, ADR-160): from now `detect()` reports no
/// backend, so commands and the CLIs that run contained elsewhere run directly on the host. Process-wide: the setting
/// belongs to the machine, and the app sets it from its config when it opens and whenever the config changes.
pub fn set_off(off: bool) {
    OFF.store(off, Ordering::Relaxed);
}

/// Whether the user turned the sandbox off (`set_off`).
pub fn is_off() -> bool {
    OFF.load(Ordering::Relaxed)
}

/// Which isolation backend is usable here. `None` sends every command to the user's approval (fail-closed, never a
/// silent unsandboxed YOLO) unless the user chose to run without one. When the user turned the sandbox off
/// (`set_off`) it reports none; `detect_os` tells what the OS itself offers.
pub fn detect() -> Availability {
    if is_off() {
        return Availability {
            backend: None,
            reason: "turned off in Settings: commands run directly on this computer, without a sandbox and without asking (YOLO)"
                .into(),
        };
    }
    detect_os()
}

/// Which isolation backend the OS offers, whether or not the user turned the sandbox off.
pub fn detect_os() -> Availability {
    #[cfg(windows)]
    {
        if mxc::available() {
            Availability {
                backend: Some(Backend::WindowsMxc),
                reason: "MXC process container: writes only in the worktree, reads limited to toolchains, network as `[sandbox] network` says"
                    .into(),
            }
        } else {
            Availability {
                backend: None,
                reason: "this Windows build cannot create MXC process containers (ADR-53); every command needs your approval"
                    .into(),
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if unix::available() {
            Availability {
                backend: Some(Backend::Bubblewrap),
                reason: "bubblewrap: writes only in the worktree, reads limited to system and toolchain folders, network as `[sandbox] network` says"
                    .into(),
            }
        } else {
            Availability {
                backend: None,
                reason: "bubblewrap is missing or cannot create user namespaces (install bubblewrap); every command needs your approval"
                    .into(),
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if unix::available() {
            Availability {
                backend: Some(Backend::Seatbelt),
                reason: "Seatbelt (experimental, ADR-55): writes only in the worktree, reads limited to system and toolchain folders, network as `[sandbox] network` says"
                    .into(),
            }
        } else {
            Availability {
                backend: None,
                reason: "sandbox-exec is not usable here; every command needs your approval".into(),
            }
        }
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Availability {
            backend: None,
            reason: "no sandbox backend for this OS; every command needs your approval".into(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("refused: {0}")]
    Refused(String),
    #[error("cannot start {program}: {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Default)]
pub struct CommandSpec {
    /// Shell command line.
    pub command: String,
    pub cwd: PathBuf,
    pub timeout: Duration,
    /// Extra environment (e.g. `AGENIUM_RUN_ID`; project secrets configured for the worktree).
    pub env: BTreeMap<String, String>,
    /// A command run in the background (ADR-129): set to stop it before its timeout; its tree is ended as on a
    /// timeout, but the output says neither `timed_out` nor an exit code.
    pub stop: Option<Arc<AtomicBool>>,
    /// Receives stdout and stderr as they are read (in order of arrival), so the output can be read while the command
    /// runs. The returned output still holds both streams in full.
    pub live: Option<Live>,
}

/// Whether a background command was asked to stop.
pub(crate) fn stopping(stop: &Option<Arc<AtomicBool>>) -> bool {
    stop.as_ref().is_some_and(|s| s.load(Ordering::Relaxed))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone)]
pub struct Executor {
    shell: Shell,
}

impl Executor {
    pub fn new(shell: Shell) -> Self {
        Self { shell }
    }

    pub fn shell(&self) -> &Shell {
        &self.shell
    }

    fn command(&self, spec: &CommandSpec) -> tokio::process::Command {
        let argv = self.argv(&spec.command);
        let mut c = tokio::process::Command::new(&argv[0]);
        c.args(&argv[1..]);
        c.current_dir(&spec.cwd)
            .env_clear()
            .envs(clean_env(&spec.env))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            // Own process group so the whole tree can be killed.
            c.process_group(0);
        }
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            c.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
        }
        c.kill_on_drop(true);
        c
    }

    /// Shell argv of a command line.
    pub fn argv(&self, command: &str) -> Vec<String> {
        match &self.shell {
            Shell::Bash(p) => vec![p.display().to_string(), "-c".into(), command.into()],
            Shell::Cmd => vec![
                "cmd".into(),
                "/D".into(),
                "/S".into(),
                "/C".into(),
                command.into(),
            ],
            Shell::PowerShell(p) => vec![
                p.display().to_string(),
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-ExecutionPolicy".into(),
                "Bypass".into(),
                "-Command".into(),
                format!("{PS_PRELUDE}{command}{PS_EPILOGUE}"),
            ],
        }
    }

    /// Environment of a sandboxed command: the clean environment plus what the container and the toolchains need
    /// (temp dir, a git config the agent may read, node without realpath of the drive root).
    #[cfg(windows)]
    fn mxc_env(
        spec: &CommandSpec,
        policy: &SandboxPolicy,
    ) -> std::io::Result<Vec<(String, String)>> {
        mxc_env(&spec.env, policy)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    async fn run_unix(spec: &CommandSpec, policy: &SandboxPolicy) -> Result<ExecOutput, ExecError> {
        let mut env = clean_env(&spec.env);
        if let Some(t) = &policy.temp_dir {
            prepare_temp_dir(policy)?;
            env.insert("TMPDIR".into(), t.display().to_string());
            // HOME stays the user's, and their ~/.gitconfig is outside the sandbox: Seatbelt answers EPERM (not
            // ENOENT as on Linux, where it is not mounted) and git stops with "unable to access", exit 128 (likely
            // cause of the macOS containment run's `ok.git_status` failure, 2026-10-26). As on Windows, the agent's
            // git reads a config of its own.
            let gc = agent_gitconfig(
                t,
                "[user]\n\tname = agenium-agent\n\temail = agent@agenium.local\n",
            )?;
            env.insert("GIT_CONFIG_GLOBAL".into(), gc.display().to_string());
            env.extend(cache_env(t)?);
        }
        if !env.contains_key("JAVA_HOME") {
            if let Some(j) = unix::host_java_home() {
                env.insert("JAVA_HOME".into(), j);
            }
        }
        env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
        let env: Vec<(String, String)> = env.into_iter().collect();
        let (command, cwd, policy, timeout) = (
            spec.command.clone(),
            spec.cwd.clone(),
            policy.clone(),
            spec.timeout,
        );
        let (stop, live) = (spec.stop.clone(), spec.live.clone());
        tokio::task::spawn_blocking(move || {
            unix::run(&command, &cwd, &env, &policy, timeout, stop, live)
        })
        .await
        .map_err(|e| ExecError::Io(std::io::Error::other(e)))?
    }

    #[cfg(windows)]
    async fn run_mxc(
        &self,
        spec: &CommandSpec,
        policy: &SandboxPolicy,
    ) -> Result<ExecOutput, ExecError> {
        let argv = self.argv(&spec.command);
        let env = Self::mxc_env(spec, policy)?;
        let (cwd, policy, timeout) = (spec.cwd.clone(), policy.clone(), spec.timeout);
        let (stop, live) = (spec.stop.clone(), spec.live.clone());
        tokio::task::spawn_blocking(move || {
            mxc::run(&argv, &cwd, &env, &policy, timeout, stop, live)
        })
        .await
        .map_err(|e| ExecError::Io(std::io::Error::other(e)))?
    }

    /// Runs the command (the caller has already obtained approval or verified the sandbox). On timeout the
    /// whole process tree is killed.
    pub async fn run(
        &self,
        spec: &CommandSpec,
        policy: &SandboxPolicy,
    ) -> Result<ExecOutput, ExecError> {
        if policy.mode != SandboxMode::FullAccess && detect().backend.is_none() {
            // Without isolation the policy cannot be enforced by the OS; callers must have asked the user.
            // (Recorded for audit; the approval gate lives in the tool layer.)
        }
        if !spec.cwd.is_dir() {
            return Err(ExecError::Refused(format!(
                "working directory {} does not exist",
                spec.cwd.display()
            )));
        }
        #[cfg(windows)]
        if policy.mode != SandboxMode::FullAccess && detect().backend == Some(Backend::WindowsMxc) {
            return self.run_mxc(spec, policy).await;
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if policy.mode != SandboxMode::FullAccess
            && matches!(
                detect().backend,
                Some(Backend::Bubblewrap | Backend::Seatbelt)
            )
        {
            return Self::run_unix(spec, policy).await;
        }
        let start = Instant::now();
        let mut child = self.command(spec).spawn().map_err(|e| ExecError::Spawn {
            program: self.shell.describe(),
            source: e,
        })?;
        let pid = child.id();
        // Windows: a job object holds the whole tree; closing it kills every process still in it, including
        // background grandchildren (no orphans, decision M2-4).
        #[cfg(windows)]
        let job = win_job::Job::new()
            .and_then(|j| {
                child
                    .raw_handle()
                    .map(|h| j.assign(h))
                    .unwrap_or(Ok(()))
                    .map(|_| j)
            })
            .ok();
        let out = child.stdout.take().expect("piped stdout");
        let err = child.stderr.take().expect("piped stderr");
        let read_out = tokio::spawn(read_all(out, spec.live.clone()));
        let read_err = tokio::spawn(read_all(err, spec.live.clone()));
        // Waits for the exit, the timeout, or a stop of a background command; the last two end the whole tree.
        let mut timed_out = false;
        let status = loop {
            if let Some(s) = child.try_wait()? {
                break s.code();
            }
            let expired = start.elapsed() >= spec.timeout;
            if expired || stopping(&spec.stop) {
                timed_out = expired;
                #[cfg(windows)]
                if let Some(j) = &job {
                    j.terminate();
                }
                if let Some(pid) = pid {
                    kill_tree(pid).await;
                }
                let _ = child.kill().await;
                break None;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        // What the command left running in its process group (a server started with `&`) ends with it, as the job
        // object ends it on Windows: on Unix it ran on after the command, unsandboxed (audit L1, 2026-10-31).
        #[cfg(unix)]
        if let Some(pgid) = pid.and_then(|p| i32::try_from(p).ok()) {
            // SAFETY: a signal to the process group the command was started in (`process_group(0)`); nothing in this
            // process is touched.
            unsafe { libc::killpg(pgid, libc::SIGKILL) };
        }
        // Grandchildren may keep the pipes open after a kill; do not wait forever for them.
        let grace = Duration::from_secs(2);
        let stdout = tokio::time::timeout(grace, read_out)
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or_default();
        let stderr = tokio::time::timeout(grace, read_err)
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or_default();
        #[cfg(windows)]
        drop(job);
        Ok(ExecOutput {
            exit_code: status,
            stdout: decode_output(&stdout),
            stderr: decode_output(&stderr),
            timed_out,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }
}

/// Environment of a sandboxed process on Windows: the clean environment plus `extra` and what the container and the
/// toolchains need (temp dir, a git config the agent may read, node without realpath of the drive root).
#[cfg(windows)]
pub(crate) fn mxc_env(
    extra: &BTreeMap<String, String>,
    policy: &SandboxPolicy,
) -> std::io::Result<Vec<(String, String)>> {
    let mut env = clean_env(extra);
    for k in ["LOCALAPPDATA", "SystemDrive", "SystemRoot"] {
        if let Ok(v) = std::env::var(k) {
            env.retain(|name, _| !name.eq_ignore_ascii_case(k));
            env.insert(k.to_owned(), v);
        }
    }
    if let Some(t) = &policy.temp_dir {
        prepare_temp_dir(policy)?;
        let gc = agent_gitconfig(
            t,
            "[user]\n\tname = agenium-agent\n\temail = agent@agenium.local\n[core]\n\tlongpaths = true\n",
        )?;
        env.insert("TEMP".into(), t.display().to_string());
        env.insert("TMP".into(), t.display().to_string());
        env.insert("GIT_CONFIG_GLOBAL".into(), gc.display().to_string());
    }
    env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    let node = env.get("NODE_OPTIONS").cloned().unwrap_or_default();
    if !node.contains("--preserve-symlinks-main") {
        env.insert(
            "NODE_OPTIONS".into(),
            format!("{node} --preserve-symlinks-main").trim().to_owned(),
        );
    }
    Ok(env.into_iter().collect())
}

/// A long-running helper process (an MCP stdio server): program + arguments started directly (no shell), clean
/// environment plus `env`, piped stdin/stdout/stderr. Pair the child with a [`TreeGuard`].
pub fn service_command(
    program: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    cwd: &Path,
) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(program);
    c.args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(clean_env(env))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        c.process_group(0);
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c.kill_on_drop(true);
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(dir: &Path) -> SandboxPolicy {
        SandboxPolicy::workspace(dir)
    }

    /// Removes the worktree's command TEMP when the test ends.
    struct RunTemp<'a>(&'a Path);

    impl Drop for RunTemp<'_> {
        fn drop(&mut self) {
            remove_run_temp_dir(self.0);
        }
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@agenium.local"])
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git {args:?}");
    }

    /// The worktree's `.git` file is the agent's to rewrite: only the git directory git made for this worktree gives a
    /// read root (the main repository's `.git`), never a folder the file names (`gitdir: <profile>` made the profile
    /// readable).
    #[test]
    fn git_read_roots_trust_only_the_worktrees_own_git_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        git(&project, &["init", "-q", "-b", "main"]);
        git(&project, &["commit", "-q", "--allow-empty", "-m", "init"]);
        let worktrees = project.join(".agenium").join("worktrees");
        let (wt, other) = (worktrees.join("T1"), worktrees.join("T2"));
        for (w, b) in [(&wt, "agenium/T1"), (&other, "agenium/T2")] {
            git(
                &project,
                &["worktree", "add", "-q", "-b", b, w.to_str().unwrap()],
            );
        }
        let main_git = std::fs::canonicalize(project.join(".git")).unwrap();
        let roots = |w: &Path| -> Vec<PathBuf> {
            git_read_roots(w)
                .iter()
                .map(|r| std::fs::canonicalize(r).unwrap())
                .collect()
        };
        assert_eq!(roots(&wt), std::slice::from_ref(&main_git));

        let genuine = std::fs::read_to_string(wt.join(".git")).unwrap();
        let point = |target: &Path| {
            std::fs::write(wt.join(".git"), format!("gitdir: {}\n", target.display())).unwrap()
        };
        // A folder outside the worktree: nothing in it points back.
        let profile = tmp.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        point(&profile);
        assert!(roots(&wt).is_empty());
        // The worktree itself, with the files git would write: its folder `.agenium/worktrees` made `.agenium` (every
        // other worktree) the common dir.
        std::fs::write(
            wt.join("gitdir"),
            format!("{}\n", wt.join(".git").display()),
        )
        .unwrap();
        std::fs::write(wt.join("commondir"), "../..\n").unwrap();
        point(&wt);
        assert!(roots(&wt).is_empty());
        // A made-up admin folder elsewhere (the agent's temp dir) that names the profile as its repository.
        let fake = tmp.path().join("tmp").join("T1");
        std::fs::create_dir_all(&fake).unwrap();
        std::fs::write(
            fake.join("gitdir"),
            format!("{}\n", wt.join(".git").display()),
        )
        .unwrap();
        std::fs::write(fake.join("commondir"), format!("{}\n", profile.display())).unwrap();
        point(&fake);
        assert!(roots(&wt).is_empty());
        // Another worktree's admin folder: it points back to that worktree.
        let other_admin = std::fs::read_to_string(other.join(".git")).unwrap();
        std::fs::write(wt.join(".git"), other_admin).unwrap();
        assert!(roots(&wt).is_empty());

        std::fs::write(wt.join(".git"), genuine).unwrap();
        assert_eq!(roots(&wt), [main_git]);
    }

    #[test]
    fn fail_closed_detection() {
        let a = detect();
        // Windows: MXC when the native API works (ADR-53); Linux: bubblewrap (ADR-54); macOS is M2.
        if cfg!(windows) {
            assert!(matches!(a.backend, None | Some(Backend::WindowsMxc)));
        } else if cfg!(target_os = "linux") {
            assert!(matches!(a.backend, None | Some(Backend::Bubblewrap)));
        } else if cfg!(target_os = "macos") {
            assert!(matches!(a.backend, None | Some(Backend::Seatbelt)));
        }
        assert!(!a.reason.is_empty());
    }

    #[tokio::test]
    async fn runs_with_clean_env_and_captures_output() {
        let tmp = tempfile::tempdir().unwrap();
        let _temp = RunTemp(tmp.path());
        std::env::set_var("AGENIUM_TEST_SECRET_CANARY", "sk-ant-CANARY");
        let ex = Executor::new(Shell::detect());
        let cmd = match ex.shell() {
            Shell::Bash(_) => "echo hello; echo oops >&2; env; exit 3",
            Shell::Cmd => "echo hello& echo oops 1>&2& set& exit /b 3",
            Shell::PowerShell(_) => "Write-Output hello; [Console]::Error.WriteLine('oops'); Get-ChildItem env: | ForEach-Object { \"$($_.Name)=$($_.Value)\" }; exit 3",
        };
        let mut extra = BTreeMap::new();
        extra.insert("AGENIUM_RUN_ID".to_string(), "run-1".to_string());
        let out = ex
            .run(
                &CommandSpec {
                    command: cmd.into(),
                    cwd: tmp.path().into(),
                    timeout: Duration::from_secs(30),
                    env: extra,
                    ..Default::default()
                },
                &policy(tmp.path()),
            )
            .await
            .unwrap();
        assert_eq!(out.exit_code, Some(3));
        assert!(out.stdout.contains("hello"));
        assert!(out.stderr.contains("oops"));
        assert!(out.stdout.contains("AGENIUM_RUN_ID=run-1"));
        assert!(
            !out.stdout.contains("CANARY"),
            "secrets never reach agent commands"
        );
    }

    /// Vietnamese text survives a command: a UTF-8 file read with the shell's own reader comes back as written
    /// (Windows PowerShell 5.1 read it as ANSI: "xin chÃ o").
    #[tokio::test]
    async fn utf8_text_survives_the_shell() {
        let tmp = tempfile::tempdir().unwrap();
        let _temp = RunTemp(tmp.path());
        std::fs::write(tmp.path().join("greeting.txt"), "xin chào\n").unwrap();
        let ex = Executor::new(Shell::detect());
        let cmd = match ex.shell() {
            Shell::Bash(_) => "cat greeting.txt",
            Shell::Cmd => "type greeting.txt",
            Shell::PowerShell(_) => "Get-Content greeting.txt",
        };
        let out = ex
            .run(
                &CommandSpec {
                    command: cmd.into(),
                    cwd: tmp.path().into(),
                    timeout: Duration::from_secs(30),
                    env: BTreeMap::new(),
                    ..Default::default()
                },
                &policy(tmp.path()),
            )
            .await
            .unwrap();
        assert!(out.stdout.contains("xin chào"), "{:?}", out.stdout);
    }

    /// Output that is not UTF-8 (a Windows program writing in the ANSI code page) is decoded with that code page;
    /// the UTF-8 lines around it stay as they are.
    #[test]
    fn output_in_the_ansi_code_page_is_still_text() {
        let s = decode_output(b"ok \xc3\xa0\n\xe0\n");
        assert!(s.starts_with("ok \u{e0}\n") && s.ends_with('\n'), "{s:?}");
        if cfg!(windows) {
            assert!(!s.contains('\u{fffd}'), "{s:?}");
        }
    }

    /// A double-byte character is decoded whole: its lead byte alone is invalid UTF-8 and its trail byte came as the
    /// next invalid run, so decoding run by run turned "中" (GBK D6 D0) into two wrong characters.
    #[test]
    fn a_double_byte_character_is_not_split() {
        let gbk = |b: &[u8]| (b == b"\xd6\xd0\n").then(|| "\u{4e2d}\n".to_owned());
        assert_eq!(decode_lines(b"ok\n\xd6\xd0\n", gbk), "ok\n\u{4e2d}\n");
        #[cfg(windows)]
        assert_eq!(
            decode_lines(b"\xd6\xd0\xce\xc4\n", |b| code_page(b, 936)),
            "\u{4e2d}\u{6587}\n"
        );
    }

    /// `[sandbox] network = true` is the default (ADR-84): a command must start with it. On Ubuntu 25.10 every such
    /// command failed, because the policy also asked bubblewrap to accept inbound connections, and where the host
    /// cannot give the sandbox a private network the command now runs without one and says so.
    #[tokio::test]
    async fn a_command_with_the_network_on_starts() {
        if detect().backend.is_none() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let _temp = RunTemp(tmp.path());
        let ex = Executor::new(Shell::detect());
        let mut p = policy(tmp.path());
        p.network = true;
        let out = ex
            .run(
                &CommandSpec {
                    command: "echo up".into(),
                    cwd: tmp.path().into(),
                    timeout: Duration::from_secs(30),
                    env: BTreeMap::new(),
                    ..Default::default()
                },
                &p,
            )
            .await
            .unwrap();
        assert_eq!(out.exit_code, Some(0), "{out:?}");
        assert!(out.stdout.contains("up"), "{out:?}");
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if unix::private_network().is_err() {
            assert!(out.stderr.contains("ran without network"), "{out:?}");
        }
    }

    /// Node resolves every `require` with `fs.realpathSync`, which reads the attributes of each folder from the
    /// drive root down to the file: inside the sandbox those parents must be listable (found by a team run whose
    /// `node --test` died with `EPERM: lstat 'C:\'`).
    #[tokio::test]
    async fn node_resolves_paths_inside_the_sandbox() {
        if detect().backend.is_none() || find_program("node").is_none() {
            eprintln!("skipped: no sandbox backend or no node here");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let _temp = RunTemp(tmp.path());
        std::fs::write(tmp.path().join("m.js"), "module.exports = 41 + 1;").unwrap();
        // A file next to the worktree: its folder may be listed, the file must stay unreadable.
        let beside = tempfile::NamedTempFile::new_in(tmp.path().parent().unwrap()).unwrap();
        std::fs::write(beside.path(), "secret").unwrap();
        std::fs::write(
            tmp.path().join("t.js"),
            format!(
                "console.log('got', require('./m.js'));
try {{ require('fs').readFileSync({:?}); console.log('LEAK') }} catch (e) {{ console.log('denied', e.code) }}",
                beside.path().to_string_lossy()
            ),
        )
        .unwrap();
        let ex = Executor::new(Shell::detect());
        let out = ex
            .run(
                &CommandSpec {
                    command: "node t.js".into(),
                    cwd: tmp.path().into(),
                    timeout: Duration::from_secs(60),
                    env: BTreeMap::new(),
                    ..Default::default()
                },
                &policy(tmp.path()),
            )
            .await
            .unwrap();
        assert!(out.stdout.contains("got 42"), "{out:?}");
        assert!(
            out.stdout.contains("denied") && !out.stdout.contains("LEAK"),
            "{out:?}"
        );
    }

    #[tokio::test]
    async fn timeout_kills_the_process_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let _temp = RunTemp(tmp.path());
        let ex = Executor::new(Shell::detect());
        let cmd = match ex.shell() {
            Shell::Bash(_) => "sleep 30 & sleep 30; echo never",
            Shell::Cmd => "ping -n 30 127.0.0.1 > nul & echo never",
            Shell::PowerShell(_) => "Start-Sleep -Seconds 30; Write-Output never",
        };
        let out = ex
            .run(
                &CommandSpec {
                    command: cmd.into(),
                    cwd: tmp.path().into(),
                    timeout: Duration::from_millis(800),
                    env: BTreeMap::new(),
                    ..Default::default()
                },
                &policy(tmp.path()),
            )
            .await
            .unwrap();
        assert!(out.timed_out);
        assert!(out.elapsed_ms < 10_000, "{}", out.elapsed_ms);
        assert!(!out.stdout.contains("never"));
    }

    /// Audit L1 (2026-10-31): unsandboxed on Unix (approval mode), what a command started in the background ran on
    /// after it ended; on Windows the job object ends it. It ends with the command.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_background_child_ends_with_its_unsandboxed_command() {
        let tmp = tempfile::tempdir().unwrap();
        let ex = Executor::new(Shell::detect());
        let mut full = policy(tmp.path());
        full.mode = SandboxMode::FullAccess;
        let out = ex
            .run(
                &CommandSpec {
                    command: "sleep 100 & echo $!".into(),
                    cwd: tmp.path().into(),
                    timeout: Duration::from_secs(30),
                    env: BTreeMap::new(),
                    ..Default::default()
                },
                &full,
            )
            .await
            .unwrap();
        let child: i32 = out.stdout.trim().parse().unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        // SAFETY: signal 0 only asks whether the process exists.
        let alive = unsafe { libc::kill(child, 0) } == 0;
        assert!(!alive, "the background sleep was ended");
    }
}
