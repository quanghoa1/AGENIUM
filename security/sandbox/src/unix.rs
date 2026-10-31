//! Linux and macOS backends through Microsoft's `mxc-sdk` (MIT) public `v1` API: bubblewrap on Linux (ADR-54),
//! Seatbelt on macOS (ADR-55).
//!
//! Allow-list model: the writable roots are read-write, the system directories, toolchain roots and explicit read
//! roots (repository `.git`, skill folders) read-only, nothing else is visible; network denied unless the policy
//! allows it (Linux: a new network namespace; macOS: the Seatbelt profile); cleared environment; the sandbox dies
//! with its parent. Codex's own helpers (`bwrap --ro-bind / /`, a Seatbelt profile reading everything) stay a
//! reference (vendor/codex-sandbox): Agenium's policy forbids reading outside the worktree.

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
/// the network in place, through Agenium's narrowing of mxc-sdk's profile ([`crate::seatbelt`]), which must hold.
pub fn private_network() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        static STATE: OnceLock<Result<(), String>> = OnceLock::new();
        STATE
            .get_or_init(|| crate::seatbelt::network_on_profile(&[], &[]).map(drop))
            .clone()
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
        // mxc-sdk 1.0.1 forwards DNS to a host loopback stub resolver (systemd-resolved's 127.0.0.53), so names
        // resolve inside the private network without touching /etc/resolv.conf (Ubuntu 25.10 VM, 2026-10-26).
        Ok(()) => (p, None),
    }
}

/// The backend for one command. macOS: no GUI, no keychain; with the network on, the profile is mxc-sdk's own with
/// its open outbound rule narrowed (no host unix sockets, [`crate::seatbelt`]). If that cannot be built the command
/// runs without network: never with more access than asked for.
#[cfg_attr(target_os = "linux", allow(unused_variables))]
fn containment(write: &[String], read: &[String], network: &mut bool) -> Containment {
    #[cfg(target_os = "linux")]
    {
        Containment::Bubblewrap
    }
    #[cfg(target_os = "macos")]
    {
        let mut config = mxc_sdk::v1::configs::SeatbeltConfig::default();
        if *network {
            match crate::seatbelt::network_on_profile(write, read) {
                Ok(profile) => config.profile_override = Some(profile),
                Err(_) => *network = false,
            }
        }
        Containment::Seatbelt(config)
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
        // A relative PATH entry (`.`, `node_modules/.bin`) would resolve against the harness's working directory,
        // often $HOME itself.
        let unsafe_root =
            p.is_relative() || p.parent().is_none() || home.as_deref() == Some(p.as_path());
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

/// The host resolver's sockets, masked in every sandbox. mxc-sdk 1.0.1 binds the resolver folders whole so that
/// `/etc/resolv.conf` can be read, and with them systemd-resolved's varlink socket: through it a command had names
/// looked up by the host with the network off (containment `net.resolver_socket`, 2026-10-30). Fixed upstream after
/// 1.0.1 (microsoft/mxc#1568); drop this once the pin moves past it. A denied socket is masked with `/dev/null`.
#[cfg(target_os = "linux")]
fn resolver_sockets() -> Vec<String> {
    use std::os::unix::fs::FileTypeExt;
    [
        "/run/systemd/resolve",
        "/run/NetworkManager",
        "/run/resolvconf",
    ]
    .iter()
    .filter_map(|d| std::fs::read_dir(d).ok())
    .flatten()
    .flatten()
    .filter(|e| e.file_type().is_ok_and(|t| t.is_socket()))
    .map(|e| e.path().display().to_string())
    .collect()
}

#[cfg(not(target_os = "linux"))]
fn resolver_sockets() -> Vec<String> {
    Vec::new()
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
    let mut network = policy.network;
    let containment = containment(&write, &read, &mut network);
    let action = if network {
        NetworkAction::Allow
    } else {
        NetworkAction::Deny
    };
    ContainerRequest {
        filesystem: Some(FilesystemPolicy {
            readwrite_paths: write,
            readonly_paths: read,
            denied_paths: resolver_sockets(),
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
        containment,
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

/// Every process a sandboxed command starts carries this variable, with a value of its own per command.
pub const STRAY_TOKEN_ENV: &str = "AGENIUM_SANDBOX_RUN";

/// A fresh token for one command (128 random bits from the standard library's hasher keys).
fn stray_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let part = || {
        std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish()
    };
    format!("{:016x}{:016x}", part(), part())
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

/// Kills this user's processes that still carry `token` once the command has ended, and returns how many. A daemon
/// (double fork + `setsid`) leaves the command's process group; on macOS nothing else ends it (no PID namespace, no
/// job object), on Linux bubblewrap's PID namespace already has. A process that clears its environment is not found;
/// Seatbelt still confines it (ADR-55).
pub fn kill_strays(token: &str) -> usize {
    let needle = format!("{STRAY_TOKEN_ENV}={token}");
    let me = std::process::id() as i32;
    let mut killed = 0;
    // A daemon may fork while it is being looked for: look again until a pass finds none.
    for _ in 0..5 {
        let mut found = 0;
        for pid in pids() {
            if pid > 1 && pid != me && carries(pid, needle.as_bytes()) {
                // SAFETY: kill(2) with a pid read from the system and a constant signal.
                if unsafe { libc::kill(pid, libc::SIGKILL) } == 0 {
                    found += 1;
                }
            }
        }
        killed += found;
        if found == 0 {
            break;
        }
    }
    killed
}

#[cfg(target_os = "linux")]
fn pids() -> Vec<i32> {
    std::fs::read_dir("/proc")
        .map(|d| {
            d.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Its environment (another user's process cannot be read, so it is never matched).
#[cfg(target_os = "linux")]
fn carries(pid: i32, needle: &[u8]) -> bool {
    std::fs::read(format!("/proc/{pid}/environ")).is_ok_and(|b| contains(&b, needle))
}

#[cfg(target_os = "macos")]
fn pids() -> Vec<i32> {
    // SAFETY: a null buffer asks for the count; the second call fills at most `buf.len()` pids.
    unsafe {
        let n = libc::proc_listallpids(std::ptr::null_mut(), 0);
        if n <= 0 {
            return Vec::new();
        }
        let mut buf = vec![0i32; n as usize + 64];
        let n = libc::proc_listallpids(
            buf.as_mut_ptr().cast(),
            (buf.len() * std::mem::size_of::<i32>()) as libc::c_int,
        );
        buf.truncate(n.max(0) as usize);
        buf
    }
}

/// Its arguments and environment (`KERN_PROCARGS2`; only this user's processes can be read).
#[cfg(target_os = "macos")]
fn carries(pid: i32, needle: &[u8]) -> bool {
    // SAFETY: sysctl(3) with fixed MIBs and buffers sized by the length it reports back.
    unsafe {
        let mut max: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>();
        let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
        if libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&mut max as *mut libc::c_int).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        ) != 0
            || max <= 0
        {
            return false;
        }
        let mut buf = vec![0u8; max as usize];
        let mut len = buf.len();
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return false;
        }
        buf.truncate(len);
        contains(&buf, needle)
    }
}

/// Runs a shell command line in the sandbox (blocking; call from a blocking thread).
pub fn run(
    command: &str,
    cwd: &Path,
    env: &[(String, String)],
    policy: &SandboxPolicy,
    timeout: Duration,
    stop: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    live: Option<crate::Live>,
) -> Result<ExecOutput, ExecError> {
    let start = Instant::now();
    let (policy, note) = effective(policy);
    let policy = &policy;
    let token = stray_token();
    let mut env = env.to_vec();
    env.push((STRAY_TOKEN_ENV.into(), token.clone()));
    let mut child =
        v1::spawn(request(command, cwd, &env, policy), Default::default()).map_err(|e| {
            ExecError::Spawn {
                program: BACKEND.into(),
                source: std::io::Error::other(e.to_string()),
            }
        })?;
    let out = child.take_stdout().map(|r| crate::reader(r, live.clone()));
    let err = child.take_stderr().map(|r| crate::reader(r, live.clone()));
    let (out_closer, err_closer) = (child.stdout_closer(), child.stderr_closer());
    let mut timed_out = false;
    let code = loop {
        if let Some(c) = child.try_wait()? {
            break Some(c);
        }
        let expired = start.elapsed() >= timeout;
        if expired || crate::stopping(&stop) {
            timed_out = expired;
            let _ = child.kill_for_timeout();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let strays = kill_strays(&token);
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
    let mut stderr_text = note.map(|n| n + "\n").unwrap_or_default();
    if strays > 0 {
        stderr_text.push_str(&format!(
            "[agenium] stopped {strays} background process(es) this command left running\n"
        ));
    }
    stderr_text.push_str(&String::from_utf8_lossy(&stderr));
    Ok(ExecOutput {
        exit_code: code,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: stderr_text,
        timed_out,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every socket of the host resolver is masked in the request, its other files are not (the containment row
    /// `net.resolver_socket` shows the effect).
    #[cfg(target_os = "linux")]
    #[test]
    fn the_host_resolver_sockets_are_masked() {
        let t = tempfile::tempdir().unwrap();
        let policy = SandboxPolicy::workspace(t.path());
        let denied = request("true", t.path(), &[], &policy)
            .filesystem
            .unwrap()
            .denied_paths;
        let socket = "/run/systemd/resolve/io.systemd.Resolve";
        assert_eq!(
            denied.iter().any(|d| d == socket),
            Path::new(socket).exists(),
            "{denied:?}"
        );
        assert!(
            !denied.iter().any(|d| d.ends_with("resolv.conf")),
            "{denied:?}"
        );
    }

    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only checks that the process exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    /// A double-forked, session-leading process (as the containment payload's daemon) outside any sandbox: found by
    /// its token and killed; a process with another token is left alone.
    #[test]
    fn a_daemon_left_running_is_found_by_its_token_and_killed() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("python3 missing: skipped");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let daemon = |token: &str, file: &str| {
            let pidfile = tmp.path().join(file);
            let script = "import os,sys,time\n\
                          if os.fork()==0:\n    os.setsid()\n    if os.fork()==0:\n        \
                          open(sys.argv[1],'w').write(str(os.getpid()))\n        time.sleep(300)\n    os._exit(0)\n\
                          os.wait()\n";
            let st = std::process::Command::new("python3")
                .args(["-c", script, pidfile.to_str().unwrap()])
                .env(STRAY_TOKEN_ENV, token)
                .status()
                .unwrap();
            assert!(st.success());
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Ok(s) = std::fs::read_to_string(&pidfile) {
                    if let Ok(p) = s.trim().parse::<i32>() {
                        return p;
                    }
                }
                assert!(Instant::now() < deadline, "the daemon did not start");
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        let (mine, other) = (stray_token(), stray_token());
        assert_ne!(mine, other);
        let a = daemon(&mine, "a.pid");
        let b = daemon(&other, "b.pid");
        assert!(alive(a) && alive(b));
        assert_eq!(kill_strays(&mine), 1);
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(a) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!alive(a), "the daemon still runs");
        assert!(alive(b), "another command's process was killed");
        assert_eq!(kill_strays(&other), 1);
        assert_eq!(kill_strays(&mine), 0);
    }
}
