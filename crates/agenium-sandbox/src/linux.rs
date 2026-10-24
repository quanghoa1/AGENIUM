//! Linux backend: bubblewrap through Microsoft's `mxc-sdk` (MIT) public `v1` API (ADR-54, spike S3).
//!
//! Allow-list model: the writable roots are bind-mounted read-write, the system directories, toolchain roots and
//! explicit read roots (repository `.git`, skill folders) read-only, nothing else is visible; fresh `/dev`, `/proc`
//! and `/tmp`; a new network namespace (`--unshare-net`, loopback only) unless the policy allows the network;
//! cleared environment; the sandbox dies with its parent. Codex's own Linux sandbox (`--ro-bind / /`, i.e. the
//! whole disk readable) stays a reference: Agenium's policy forbids reading outside the worktree.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use mxc_sdk::v1::policy::{
    NetworkAction, NetworkEgressPolicy, NetworkIngressPolicy, NetworkPolicy,
};
use mxc_sdk::v1::{self, ContainerRequest, Containment, FilesystemPolicy};

use crate::{ExecError, ExecOutput, SandboxMode, SandboxPolicy};

/// Whether bubblewrap can create a sandbox here (user namespaces available; cached).
pub fn available() -> bool {
    static USABLE: OnceLock<bool> = OnceLock::new();
    *USABLE.get_or_init(|| {
        v1::available_backends()
            .iter()
            .any(|b| b.backend == "bubblewrap")
    })
}

/// System directories every process needs (loader, libraries, certificates, locale data).
pub fn platform_read_roots() -> Vec<PathBuf> {
    [
        "/usr", "/bin", "/sbin", "/lib", "/lib64", "/lib32", "/etc", "/opt",
    ]
    .iter()
    .map(PathBuf::from)
    .filter(|p| p.exists())
    .collect()
}

/// Toolchains on PATH outside the system directories (e.g. `~/.cargo/bin` -> Cargo's `bin`, `registry`, `git`;
/// `~/.rustup`; `~/.local/bin`'s parent is NOT added). Never `$HOME` or `/`.
pub fn toolchain_read_roots() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".cargo")));
    let system = platform_read_roots();
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        let unsafe_root = p.parent().is_none() || home.as_deref() == Some(p.as_path());
        if p.is_dir()
            && !unsafe_root
            && !system.iter().any(|s| p.starts_with(s))
            && !out.contains(&p)
        {
            out.push(p);
        }
    };
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            if cargo_home.as_ref().is_some_and(|c| d.starts_with(c)) {
                continue;
            }
            push(d);
        }
    }
    if let Some(c) = &cargo_home {
        for sub in ["bin", "registry", "git"] {
            push(c.join(sub));
        }
        let rustup = std::env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .or_else(|| c.parent().map(|p| p.join(".rustup")));
        if let Some(r) = rustup {
            push(r);
        }
    }
    out
}

fn paths(v: impl IntoIterator<Item = PathBuf>) -> Vec<String> {
    let mut out: Vec<String> = v
        .into_iter()
        .filter(|p| p.exists())
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p).display().to_string())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// `bash -c '<command>'` inside the sandbox's `sh -c` (bash features keep working).
fn bash_line(command: &str) -> String {
    format!("exec bash -c '{}'", command.replace('\'', r"'\''"))
}

/// The container request for one command (exposed for tests).
pub fn request(
    command: &str,
    cwd: &Path,
    env: &[(String, String)],
    policy: &SandboxPolicy,
) -> ContainerRequest {
    let mut write: Vec<PathBuf> = policy.writable_roots.clone();
    if let Some(t) = &policy.temp_dir {
        write.push(t.clone());
    }
    let mut read = platform_read_roots();
    read.extend(policy.read_roots.iter().cloned());
    if policy.mode == SandboxMode::ReadOnly {
        read.append(&mut write);
    }
    let write = paths(write);
    let read: Vec<String> = paths(read)
        .into_iter()
        .filter(|r| !write.contains(r))
        .collect();
    let action = if policy.network {
        NetworkAction::Allow
    } else {
        NetworkAction::Deny
    };
    ContainerRequest {
        filesystem: Some(FilesystemPolicy {
            readwrite_paths: write,
            readonly_paths: read,
            ..Default::default()
        }),
        network: Some(NetworkPolicy {
            egress: Some(NetworkEgressPolicy {
                default: Some(action),
                ..Default::default()
            }),
            ingress: Some(NetworkIngressPolicy {
                default: Some(action),
                host_loopback: Some(action),
            }),
            ..Default::default()
        }),
        containment: Containment::Bubblewrap,
        working_directory: Some(
            std::fs::canonicalize(cwd)
                .unwrap_or(cwd.to_path_buf())
                .display()
                .to_string(),
        ),
        environment: Some(env.to_vec()),
        inherit_default_environment: Some(false),
        ..ContainerRequest::new(bash_line(command))
    }
}

fn reader(mut r: Box<dyn Read + Send>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = r.read_to_end(&mut b);
        b
    })
}

/// Runs a shell command line in the sandbox (blocking; call from a blocking thread).
pub fn run(
    command: &str,
    cwd: &Path,
    env: &[(String, String)],
    policy: &SandboxPolicy,
    timeout: Duration,
) -> Result<ExecOutput, ExecError> {
    let start = Instant::now();
    let mut child =
        v1::spawn(request(command, cwd, env, policy), Default::default()).map_err(|e| {
            ExecError::Spawn {
                program: "bwrap".into(),
                source: std::io::Error::other(e.to_string()),
            }
        })?;
    let out = child.take_stdout().map(reader);
    let err = child.take_stderr().map(reader);
    let (out_closer, err_closer) = (child.stdout_closer(), child.stderr_closer());
    let mut timed_out = false;
    let code = loop {
        if let Some(c) = child.try_wait()? {
            break Some(c);
        }
        if start.elapsed() >= timeout {
            timed_out = true;
            let _ = child.kill_for_timeout();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let grace = Instant::now() + Duration::from_secs(2);
    let join = |h: Option<std::thread::JoinHandle<Vec<u8>>>,
                closer: &Option<v1::StreamCloser>|
     -> Vec<u8> {
        let Some(h) = h else { return Vec::new() };
        while !h.is_finished() && Instant::now() < grace {
            std::thread::sleep(Duration::from_millis(20));
        }
        if !h.is_finished() {
            if let Some(c) = closer {
                c.close();
            }
        }
        h.join().unwrap_or_default()
    };
    let stdout = join(out, &out_closer);
    let stderr = join(err, &err_closer);
    Ok(ExecOutput {
        exit_code: code,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        timed_out,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}
