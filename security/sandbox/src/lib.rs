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
    /// Read-only roots beyond the platform folders (toolchains, the repository's `.git`, skill folders).
    #[serde(default)]
    pub read_roots: Vec<PathBuf>,
    /// Writable temporary directory outside the worktree (TEMP/TMP of the command); created on demand.
    #[serde(default)]
    pub temp_dir: Option<PathBuf>,
}

impl SandboxPolicy {
    /// Workspace-write policy for a ticket worktree: the worktree is writable, the toolchains and the
    /// repository's git directory (a worktree's git dir lives outside it) are readable, network off.
    pub fn workspace(root: &Path) -> SandboxPolicy {
        let mut read_roots = toolchain_read_roots();
        read_roots.extend(git_read_roots(root));
        SandboxPolicy {
            mode: SandboxMode::WorkspaceWrite,
            writable_roots: vec![root.to_path_buf()],
            network: false,
            read_roots,
            temp_dir: Some(run_temp_dir(root)),
        }
    }
}

/// Toolchain folders a sandboxed command may read (Windows: see `mxc::toolchain_read_roots`).
pub fn toolchain_read_roots() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        mxc::toolchain_read_roots()
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        unix::toolchain_read_roots()
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}

/// The git directories a worktree needs: `.git` is a file `gitdir: <repo>/.git/worktrees/<name>` whose
/// `commondir` is the repository's `.git`.
///
/// That file is inside the writable root: an agent can rewrite it between runs (`gitdir: C:\Users\me` made the whole
/// profile readable). So the directory it names is only believed when git itself vouches for it: it lies outside the
/// worktree, its `gitdir` file points back to this worktree's `.git`, and it is `<common>/worktrees/<its name>` of
/// the repository its `commondir` names. The agent cannot write outside the worktree and its temp folders, so it can
/// only name the directory git made for this worktree; anything else gives no git read root (git then fails inside
/// the sandbox instead of reading elsewhere).
pub fn git_read_roots(root: &Path) -> Vec<PathBuf> {
    let canon = |p: &Path| std::fs::canonicalize(p).ok();
    let (Ok(text), Some(root)) = (std::fs::read_to_string(root.join(".git")), canon(root)) else {
        return Vec::new(); // a normal repository: .git is inside the writable root
    };
    let Some(gitdir) = text
        .trim()
        .strip_prefix("gitdir:")
        .and_then(|g| canon(&root.join(g.trim())))
    else {
        return Vec::new();
    };
    // Outside the worktree: inside it (`gitdir: .`, whose parent folder is `.agenium/worktrees`) the agent writes the
    // `gitdir` and `commondir` files itself.
    if gitdir.starts_with(&root) {
        return Vec::new();
    }
    let points_back = std::fs::read_to_string(gitdir.join("gitdir"))
        .ok()
        .and_then(|b| canon(&gitdir.join(b.trim())))
        .is_some_and(|b| b == root.join(".git"));
    let common = std::fs::read_to_string(gitdir.join("commondir"))
        .ok()
        .and_then(|c| canon(&gitdir.join(c.trim())));
    let Some(common) = common.filter(|c| {
        points_back
            && gitdir.file_name().is_some_and(|name| {
                canon(&c.join("worktrees").join(name)).as_deref() == Some(gitdir.as_path())
            })
    }) else {
        return Vec::new();
    };
    vec![PathBuf::from(
        common
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_owned(),
    )]
}

/// Whether sandboxed commands asked to have the network get it on this machine; `Err` says why not (Linux without a
/// private network: they run without one, ADR-95).
pub fn network_available() -> Result<(), String> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        unix::private_network()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Ok(())
    }
}

/// What `[sandbox] network = true` gives sandboxed commands on this machine, in one line (`agenium doctor`).
pub fn network_report() -> String {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        match unix::private_network() {
            Err(why) => {
                format!("commands run WITHOUT network, and CLI agents cannot run sandboxed: {why}")
            }
            Ok(()) => "private network (outbound only; no inbound, no host loopback)".into(),
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        "network through the container's own policy (Windows MXC)".into()
    }
}

/// Per-worktree temporary directory outside the worktree (so auto-commit never sees it): the TEMP/TMP of the
/// worktree's commands. Keyed by the worktree path, spelled either way (separators; case on Windows).
pub fn run_temp_dir(root: &Path) -> PathBuf {
    let mut key = root.to_string_lossy().replace('\\', "/");
    key = key
        .trim_start_matches("//?/")
        .trim_end_matches('/')
        .to_owned();
    if cfg!(windows) {
        key = key.to_lowercase();
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    std::env::temp_dir()
        .join("agenium")
        .join(format!("{h:016x}"))
}

/// Deletes the worktree's temporary directory when the worktree goes away. Builds and package managers fill it, and
/// it lives under the system TEMP (on Windows usually the small system drive); nothing removed it before, so every
/// ticket left one behind. Best effort: a file still open is left for the next removal.
pub fn remove_run_temp_dir(root: &Path) {
    let dir = run_temp_dir(root);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

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

/// Which isolation backend is usable here. `None` sends every command to the user's approval (fail-closed, never a
/// silent unsandboxed YOLO).
pub fn detect() -> Availability {
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

/// Shell used for `shell` tool commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Shell {
    /// `bash -c` (Linux/macOS, or Git Bash on Windows).
    Bash(PathBuf),
    /// `cmd /C` on Windows when no bash is found.
    Cmd,
    /// PowerShell on Windows when commands run in the MXC container: Git Bash (MSYS runtime) cannot start there,
    /// it needs a global named-object directory the container denies (S2).
    PowerShell(PathBuf),
}

/// Prelude of sandboxed PowerShell commands: UTF-8 output, files read as UTF-8 (Windows PowerShell 5.1 reads them as
/// ANSI: "xin chào" came back "xin chÃ o"), no progress noise; the epilogue passes native exit codes through. Writing
/// keeps PowerShell's defaults: 5.1 would add a BOM to UTF-8.
const PS_PRELUDE: &str = "$ProgressPreference='SilentlyContinue'; $OutputEncoding=[Console]::OutputEncoding=New-Object System.Text.UTF8Encoding $false; foreach ($c in 'Get-Content','Select-String','Import-Csv') { $PSDefaultParameterValues[\"${c}:Encoding\"]='UTF8' };\n";
const PS_EPILOGUE: &str = "\n$__agenium_ok = $?; if ($LASTEXITCODE) { exit $LASTEXITCODE } elseif (-not $__agenium_ok) { exit 1 }";

impl Shell {
    pub fn detect() -> Shell {
        #[cfg(windows)]
        if mxc::available() {
            return Shell::powershell();
        }
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

    /// PowerShell 7 (`pwsh`) when installed, otherwise Windows PowerShell 5.1.
    pub fn powershell() -> Shell {
        if let Some(p) = find_program("pwsh") {
            return Shell::PowerShell(p);
        }
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        Shell::PowerShell(
            PathBuf::from(root)
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe"),
        )
    }

    pub fn describe(&self) -> String {
        match self {
            Shell::Bash(p) => format!("bash ({})", p.display()),
            Shell::Cmd => "cmd.exe".into(),
            Shell::PowerShell(p) => {
                format!("PowerShell ({}); write PowerShell, not bash", p.display())
            }
        }
    }
}

/// A command's output as text. It is UTF-8 where the environment and the prelude could ask for it; bytes that are not
/// (a Windows program writing in the ANSI code page) are decoded with that code page rather than shown as U+FFFD.
pub fn decode_output(bytes: &[u8]) -> String {
    decode_lines(bytes, ansi)
}

/// Valid UTF-8 stays as it is. Otherwise each line that is not UTF-8 is decoded whole with `ansi`: decoding every
/// invalid run on its own split the characters of a double-byte code page (GBK, Shift-JIS: a lead byte alone is
/// invalid UTF-8, its trail byte the next run). No byte of such a character is a line feed, so a line keeps them
/// whole, and the UTF-8 lines of a mixed stream stay readable.
fn decode_lines(bytes: &[u8], ansi: impl Fn(&[u8]) -> Option<String>) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }
    let mut out = String::with_capacity(bytes.len());
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        match std::str::from_utf8(line) {
            Ok(s) => out.push_str(s),
            Err(_) => match ansi(line) {
                Some(s) => out.push_str(&s),
                None => out.push_str(&String::from_utf8_lossy(line)),
            },
        }
    }
    out
}

#[cfg(windows)]
fn ansi(bytes: &[u8]) -> Option<String> {
    code_page(bytes, windows_sys::Win32::Globalization::CP_ACP)
}

#[cfg(windows)]
fn code_page(bytes: &[u8], cp: u32) -> Option<String> {
    use windows_sys::Win32::Globalization::MultiByteToWideChar;
    let len = i32::try_from(bytes.len()).ok()?;
    // SAFETY: the pointers and lengths describe `bytes` and `wide`, which outlive the calls.
    let n = unsafe { MultiByteToWideChar(cp, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0) };
    if n <= 0 {
        return None;
    }
    let mut wide = vec![0u16; n as usize];
    let m = unsafe { MultiByteToWideChar(cp, 0, bytes.as_ptr(), len, wide.as_mut_ptr(), n) };
    (m > 0).then(|| String::from_utf16_lossy(&wide[..m as usize]))
}

#[cfg(not(windows))]
fn ansi(_: &[u8]) -> Option<String> {
    None
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
    // Python writes UTF-8 to a pipe (on Windows it used the ANSI code page: "xin ch\u{fffd}o").
    env.insert("PYTHONUTF8".into(), "1".into());
    env.insert("PYTHONIOENCODING".into(), "utf-8".into());
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
            std::fs::create_dir_all(t)?;
            env.insert("TMPDIR".into(), t.display().to_string());
            // HOME stays the user's, and their ~/.gitconfig is outside the sandbox: Seatbelt answers EPERM (not
            // ENOENT as on Linux, where it is not mounted) and git stops with "unable to access", exit 128 (likely
            // cause of the macOS containment run's `ok.git_status` failure, 2026-10-26). As on Windows, the agent's
            // git reads a config of its own.
            let gc = t.join("agent.gitconfig");
            if !gc.is_file() {
                std::fs::write(
                    &gc,
                    "[user]\n\tname = agenium-agent\n\temail = agent@agenium.local\n",
                )?;
            }
            env.insert("GIT_CONFIG_GLOBAL".into(), gc.display().to_string());
        }
        env.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
        let env: Vec<(String, String)> = env.into_iter().collect();
        let (command, cwd, policy, timeout) = (
            spec.command.clone(),
            spec.cwd.clone(),
            policy.clone(),
            spec.timeout,
        );
        tokio::task::spawn_blocking(move || unix::run(&command, &cwd, &env, &policy, timeout))
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
        tokio::task::spawn_blocking(move || mxc::run(&argv, &cwd, &env, &policy, timeout))
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
        std::fs::create_dir_all(t)?;
        let gc = t.join("agent.gitconfig");
        if !gc.is_file() {
            std::fs::write(&gc, "[user]\n\tname = agenium-agent\n\temail = agent@agenium.local\n[core]\n\tlongpaths = true\n")?;
        }
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
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW: the desktop app has no console
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
