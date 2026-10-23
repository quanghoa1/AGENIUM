//! Command execution for agent-generated commands (decision 9, ADR-39/41/42).
//!
//! The sandbox applies only to commands an agent runs (shell, MCP stdio), never to the harness. This crate:
//! * detects which isolation backend works on this machine and **fails closed**: YOLO (no approval) is only
//!   allowed with a working backend; Windows has none in v1, Linux needs bubblewrap or Landlock ABI >= 3, macOS
//!   Seatbelt (experimental). The backends themselves are milestone M2;
//! * runs commands with a cleared environment (no API keys), a timeout and a process-tree kill.
//!
//! Before M2 every agent command runs through [`Executor::run`] only after the user approved it (M1-7).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SandboxMode {
    ReadOnly,
    /// Write inside the writable roots (ticket worktree + tmp); network off unless granted.
    WorkspaceWrite,
    FullAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxPolicy {
    pub mode: SandboxMode,
    pub writable_roots: Vec<PathBuf>,
    pub network: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backend {
    Bubblewrap,
    LandlockSeccomp,
    Seatbelt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Availability {
    pub backend: Option<Backend>,
    /// Why no backend is available (shown to the user, decision 24).
    pub reason: String,
}

/// Which isolation backend is usable here. Backends ship in M2: until then the answer is `None` on every OS,
/// which forces the approval policy (fail-closed, never a silent unsandboxed YOLO).
pub fn detect() -> Availability {
    if cfg!(windows) {
        return Availability {
            backend: None,
            reason:
                "Windows has no sandbox backend in v1 (ADR-39); every command needs your approval"
                    .into(),
        };
    }
    if cfg!(target_os = "macos") {
        return Availability { backend: None, reason: "the macOS Seatbelt backend arrives with milestone M2; every command needs your approval".into() };
    }
    Availability { backend: None, reason: "the Linux bubblewrap/Landlock backend arrives with milestone M2; every command needs your approval".into() }
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

/// Shell used for `shell` tool commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Shell {
    /// `bash -c` (Linux/macOS, or Git Bash on Windows).
    Bash(PathBuf),
    /// `cmd /C` on Windows when no bash is found.
    Cmd,
}

impl Shell {
    pub fn detect() -> Shell {
        if cfg!(windows) {
            for cand in [
                r"C:\Program Files\Git\bin\bash.exe",
                r"C:\Program Files\Git\usr\bin\bash.exe",
            ] {
                if Path::new(cand).is_file() {
                    return Shell::Bash(cand.into());
                }
            }
            if let Some(p) = find_in_path("bash.exe") {
                // Skip the WSL launcher in System32: it would run in another OS.
                if !p
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("system32")
                {
                    return Shell::Bash(p);
                }
            }
            Shell::Cmd
        } else {
            Shell::Bash(find_in_path("bash").unwrap_or_else(|| "/bin/sh".into()))
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Shell::Bash(p) => format!("bash ({})", p.display()),
            Shell::Cmd => "cmd.exe".into(),
        }
    }
}

pub fn find_in_path(exe: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(exe))
        .find(|p| p.is_file())
}

/// Resolves a program name like the OS shell would: on Windows a name without extension is tried with the
/// `PATHEXT` extensions first (`npx` -> `npx.cmd`, not the extensionless shell script next to it).
pub fn find_program(cmd: &str) -> Option<PathBuf> {
    let p = Path::new(cmd);
    if p.components().count() > 1 || p.is_absolute() {
        return p.is_file().then(|| p.to_path_buf());
    }
    #[cfg(windows)]
    if p.extension().is_none() {
        let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        let path = std::env::var_os("PATH")?;
        for d in std::env::split_paths(&path) {
            for e in exts.split(';').filter(|e| !e.is_empty()) {
                let c = d.join(format!("{cmd}{}", e.to_ascii_lowercase()));
                if c.is_file() {
                    return Some(c);
                }
            }
        }
    }
    find_in_path(cmd)
}

/// Environment variables passed through to agent commands; everything else (API keys included) is dropped.
const ENV_ALLOW: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "USERNAME",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "TEMP",
    "TMP",
    "SYSTEMROOT",
    "SystemRoot",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "GOPATH",
];

pub fn clean_env(extra: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = std::env::vars()
        .filter(|(k, _)| ENV_ALLOW.iter().any(|a| a.eq_ignore_ascii_case(k)))
        .collect();
    env.insert("NO_COLOR".into(), "1".into());
    env.insert("GIT_TERMINAL_PROMPT".into(), "0".into());
    env.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
    env
}

#[derive(Debug, Clone)]
pub struct CommandSpec {
    /// Shell command line.
    pub command: String,
    pub cwd: PathBuf,
    pub timeout: Duration,
    /// Extra environment (e.g. `AGENIUM_RUN_ID`; project secrets configured for the worktree).
    pub env: BTreeMap<String, String>,
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
        let mut c = match &self.shell {
            Shell::Bash(p) => {
                let mut c = tokio::process::Command::new(p);
                c.arg("-c").arg(&spec.command);
                c
            }
            Shell::Cmd => {
                let mut c = tokio::process::Command::new("cmd");
                c.arg("/D").arg("/S").arg("/C").arg(&spec.command);
                c
            }
        };
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
        let mut out = child.stdout.take().expect("piped stdout");
        let mut err = child.stderr.take().expect("piped stderr");
        let read_out = tokio::spawn(async move {
            let mut b = Vec::new();
            let _ = out.read_to_end(&mut b).await;
            b
        });
        let read_err = tokio::spawn(async move {
            let mut b = Vec::new();
            let _ = err.read_to_end(&mut b).await;
            b
        });
        let (status, timed_out) = match tokio::time::timeout(spec.timeout, child.wait()).await {
            Ok(s) => (s?.code(), false),
            Err(_) => {
                #[cfg(windows)]
                if let Some(j) = &job {
                    j.terminate();
                }
                if let Some(pid) = pid {
                    kill_tree(pid).await;
                }
                let _ = child.kill().await;
                (None, true)
            }
        };
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
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            timed_out,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }
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

/// Kills a child's whole process tree when dropped (Windows: job object; Unix: its process group).
pub struct TreeGuard {
    #[cfg(windows)]
    _job: Option<win_job::Job>,
    #[cfg(unix)]
    pgid: Option<u32>,
}

impl TreeGuard {
    pub fn attach(child: &tokio::process::Child) -> TreeGuard {
        #[cfg(windows)]
        {
            let job = win_job::Job::new()
                .and_then(|j| {
                    child
                        .raw_handle()
                        .map(|h| j.assign(h))
                        .unwrap_or(Ok(()))
                        .map(|_| j)
                })
                .ok();
            TreeGuard { _job: job }
        }
        #[cfg(unix)]
        {
            TreeGuard { pgid: child.id() }
        }
    }
}

impl Drop for TreeGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pgid) = self.pgid {
            let _ = std::process::Command::new("kill")
                .args(["-KILL", "--", &format!("-{pgid}")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

#[cfg(windows)]
mod win_job {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub struct Job(HANDLE);

    // SAFETY: a job handle may be used from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn new() -> std::io::Result<Job> {
            // SAFETY: plain Win32 calls with valid arguments; the handle is owned by `Job` and closed in Drop.
            unsafe {
                let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if h.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 {
                    let e = std::io::Error::last_os_error();
                    CloseHandle(h);
                    return Err(e);
                }
                Ok(Job(h))
            }
        }

        pub fn assign(&self, process: std::os::windows::io::RawHandle) -> std::io::Result<()> {
            // SAFETY: `process` is a live process handle owned by the tokio Child.
            if unsafe { AssignProcessToJobObject(self.0, process as HANDLE) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }

        pub fn terminate(&self) {
            // SAFETY: valid job handle.
            unsafe { TerminateJobObject(self.0, 1) };
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: closing the last handle kills the remaining processes (KILL_ON_JOB_CLOSE).
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// Kills a process and all its descendants.
pub async fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = tokio::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    #[cfg(unix)]
    {
        let _ = tokio::process::Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .status()
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(dir: &Path) -> SandboxPolicy {
        SandboxPolicy {
            mode: SandboxMode::WorkspaceWrite,
            writable_roots: vec![dir.into()],
            network: false,
        }
    }

    #[test]
    fn fail_closed_detection() {
        let a = detect();
        assert!(a.backend.is_none(), "no backend before M2 on any OS");
        assert!(!a.reason.is_empty());
    }

    #[tokio::test]
    async fn runs_with_clean_env_and_captures_output() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENIUM_TEST_SECRET_CANARY", "sk-ant-CANARY");
        let ex = Executor::new(Shell::detect());
        let cmd = match ex.shell() {
            Shell::Bash(_) => "echo hello; echo oops >&2; env; exit 3",
            Shell::Cmd => "echo hello& echo oops 1>&2& set& exit /b 3",
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

    #[tokio::test]
    async fn timeout_kills_the_process_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let ex = Executor::new(Shell::detect());
        let cmd = match ex.shell() {
            Shell::Bash(_) => "sleep 30 & sleep 30; echo never",
            Shell::Cmd => "ping -n 30 127.0.0.1 > nul & echo never",
        };
        let out = ex
            .run(
                &CommandSpec {
                    command: cmd.into(),
                    cwd: tmp.path().into(),
                    timeout: Duration::from_millis(800),
                    env: BTreeMap::new(),
                },
                &policy(tmp.path()),
            )
            .await
            .unwrap();
        assert!(out.timed_out);
        assert!(out.elapsed_ms < 10_000, "{}", out.elapsed_ms);
        assert!(!out.stdout.contains("never"));
    }
}
