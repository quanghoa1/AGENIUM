//! Linux and macOS backends through Microsoft's `mxc-sdk` (MIT) public `v1` API: bubblewrap on Linux (ADR-54),
//! Seatbelt on macOS (ADR-55).
//!
//! Allow-list model: the writable roots are read-write, the system directories, toolchain roots and explicit read
//! roots (repository `.git`, skill folders) read-only, nothing else is visible; network denied unless the policy
//! allows it (Linux: a new network namespace; macOS: the Seatbelt profile); cleared environment; the sandbox dies
//! with its parent. Codex's own helpers (`bwrap --ro-bind / /`, a Seatbelt profile reading everything) stay a
//! reference (vendor/codex-sandbox): Agenium's policy forbids reading outside the worktree.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use mxc_sdk::v1::policy::{
    NetworkAction, NetworkEgressPolicy, NetworkIngressPolicy, NetworkPolicy,
};
use mxc_sdk::v1::{self, ContainerRequest, Containment, FilesystemPolicy};

use crate::{ExecError, ExecOutput, SandboxMode, SandboxPolicy};

#[cfg(target_os = "linux")]
const BACKEND: &str = "bubblewrap";
#[cfg(target_os = "macos")]
const BACKEND: &str = "seatbelt";

/// Whether the OS sandbox works here (Linux: bwrap + user namespaces; macOS: `sandbox-exec`); cached.
pub fn available() -> bool {
    static USABLE: OnceLock<bool> = OnceLock::new();
    *USABLE.get_or_init(|| {
        v1::available_backends()
            .iter()
            .any(|b| b.backend == BACKEND)
    })
}

/// Can a command with the network on get a private network here? `Err` says why not, in words for the user (cached).
/// Linux: bubblewrap gives it its own network namespace reached through slirp4netns, which needs slirp4netns and
/// unprivileged user + network namespaces — Ubuntu 24.04+ refuses those by default (AppArmor). macOS: Seatbelt opens
/// the network in place, so it always can.
pub fn private_network() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        static STATE: OnceLock<Result<(), String>> = OnceLock::new();
        STATE
            .get_or_init(|| {
                if crate::find_in_path("slirp4netns").is_none() {
                    return Err("slirp4netns is not installed (sudo apt install slirp4netns)".into());
                }
                // The probe mxc-sdk runs before it starts slirp: a namespace that cannot map the user (AppArmor lets a
                // bare `unshare --user --net` through, then refuses the uid map) is no namespace for the sandbox.
                let allowed = std::process::Command::new("unshare")
                    .args(["--user", "--map-current-user", "--keep-caps", "--net", "--", "sh", "-c", ":"])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success());
                if !allowed {
                    return Err("this host refuses unprivileged user and network namespaces (Ubuntu 24.04+: \
                         AppArmor's kernel.apparmor_restrict_unprivileged_userns=1; allow them with an AppArmor \
                         profile for Agenium, or set that sysctl to 0)"
                        .into());
                }
                Ok(())
            })
            .clone()
    }
}

/// A host whose `/etc/resolv.conf` names only its own loopback stub (systemd-resolved's 127.0.0.53) resolves no
/// names inside the sandbox's private network, where that address is the sandbox's own loopback.
pub fn private_dns_problem() -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let conf = std::fs::read_to_string("/etc/resolv.conf").ok()?;
    let servers: Vec<&str> = conf
        .lines()
        .filter_map(|l| l.trim().strip_prefix("nameserver"))
        .map(str::trim)
        .collect();
    let local = |s: &&str| s.starts_with("127.") || *s == "::1";
    (!servers.is_empty() && servers.iter().all(local)).then(|| {
        format!(
            "names do not resolve inside the sandbox: /etc/resolv.conf names only {} (the host's local resolver, \
             e.g. systemd-resolved); fix: sudo ln -sf /run/systemd/resolve/resolv.conf /etc/resolv.conf",
            servers.join(", ")
        )
    })
}

/// The policy a command actually runs with, and the line that tells the agent what changed: with the network on but
/// no private network possible, the command runs without network rather than not at all (never with more access
/// than asked for; the owner's decision of 2026-10-26).
fn effective(policy: &SandboxPolicy) -> (SandboxPolicy, Option<String>) {
    let mut p = policy.clone();
    if !p.network {
        return (p, None);
    }
    match private_network() {
        Err(why) => {
            p.network = false;
            (
                p,
                Some(format!("[agenium] this command ran without network: {why}")),
            )
        }
        Ok(()) => (p, private_dns_problem().map(|d| format!("[agenium] {d}"))),
    }
}

fn containment() -> Containment {
    #[cfg(target_os = "linux")]
    {
        Containment::Bubblewrap
    }
    #[cfg(target_os = "macos")]
    {
        // No GUI, no keychain, no nested PTYs.
        Containment::Seatbelt(mxc_sdk::v1::configs::SeatbeltConfig::default())
    }
}

/// System directories every process needs (loader, libraries, certificates, locale data; on macOS the system
/// frameworks, the command line tools and Homebrew).
pub fn platform_read_roots() -> Vec<PathBuf> {
    #[cfg(target_os = "linux")]
    let roots: &[&str] = &[
        "/usr", "/bin", "/sbin", "/lib", "/lib64", "/lib32", "/etc", "/opt",
    ];
    #[cfg(target_os = "macos")]
    let roots: &[&str] = &[
        "/usr",
        "/bin",
        "/sbin",
        "/System",
        "/Library/Developer/CommandLineTools",
        "/Applications/Xcode.app",
        "/opt/homebrew",
        "/usr/local",
        "/private/etc",
        "/etc",
        "/private/var/db/timezone",
    ];
    roots
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
    // `network` opens the way out only. Nothing reaches into the sandbox and it cannot reach the host's loopback
    // (local databases, Agenium's own ports): bubblewrap refuses an accepting ingress posture outright, and with it
    // every command ran with the default `[sandbox] network = true` failed to start (Ubuntu 25.10, 2026-10-26).
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
                default: Some(NetworkAction::Deny),
                host_loopback: Some(NetworkAction::Deny),
            }),
            ..Default::default()
        }),
        containment: containment(),
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
    let (policy, note) = effective(policy);
    let policy = &policy;
    let mut child =
        v1::spawn(request(command, cwd, env, policy), Default::default()).map_err(|e| {
            ExecError::Spawn {
                program: BACKEND.into(),
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
    let mut stderr_text = note
        .map(|n| {
            n + "
"
        })
        .unwrap_or_default();
    stderr_text.push_str(&String::from_utf8_lossy(&stderr));
    Ok(ExecOutput {
        exit_code: code,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: stderr_text,
        timed_out,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}
