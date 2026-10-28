//! Windows backend: Microsoft MXC process containers through `mxc-sdk` (MIT), the native path Codex's
//! `codex-mxc-sandbox` uses (S2, ADR-53). No Codex binary, no administrator rights, no extra users.
//!
//! The parts adapted from OpenAI Codex (availability probe, container-policy shape, platform folders) live in
//! `crate::codex_derived`; this file is Agenium's own glue (toolchain roots, paths, environment, run loop).
//!
//! Policy: the writable roots (ticket worktree, per-run temp dir) are read-write; the Windows platform folders,
//! toolchain roots and explicit read roots (repository `.git`, skill folders) are read-only; everything else is
//! invisible; network egress, ingress and host loopback are denied unless the policy allows the network.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mxc_sdk::mxc_common::logger::{Logger, Mode};
use mxc_sdk::mxc_common::models::ExecutionRequest;
use mxc_sdk::mxc_common::sandbox_process::{SandboxBackend, StdioMode};
use mxc_sdk::process_container_common::base_container_runner::BaseContainerRunner;

use crate::codex_derived;
pub use crate::codex_derived::mxc_policy::platform_read_roots;
pub use crate::codex_derived::mxc_probe::available;
use crate::{ExecError, ExecOutput, SandboxMode, SandboxPolicy};

// tracelogging (pulled in by mxc-sdk) leaves the ETW import library to the application on GNU targets.
#[link(name = "advapi32")]
unsafe extern "system" {}

/// A path in the form used to compare folders: long (no 8.3 short names, junctions resolved, when it exists) and
/// without case, as Windows names them (`C:\Users\Me`, `c:\users\me` and `C:\Users\ME~1` are one folder).
fn fold(p: &Path) -> PathBuf {
    PathBuf::from(long_path(p).to_string_lossy().to_lowercase())
}

/// A drive root, the user profile, or a relative PATH entry (`.`, `.\bin`, `\tools`): one would resolve against the
/// harness's working directory, often the profile itself.
fn is_unsafe_root(p: &Path, profile: Option<&Path>) -> bool {
    !p.is_absolute()
        || p.parent().is_none()
        || profile.is_some_and(|h| fold(p) == fold(h))
        || p.components().count() <= 1
}

/// Read-only roots for the toolchains on PATH: every PATH directory and, for `bin`/`cmd`/`Scripts` folders, the
/// toolchain root above it (Git, Python, MSYS2). Never the user profile or a drive root. Cargo is special: only
/// `bin`, `registry` and `git` of `CARGO_HOME` (its `credentials.toml` holds a token) plus `RUSTUP_HOME`.
pub fn toolchain_read_roots() -> Vec<PathBuf> {
    let profile = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| profile.as_ref().map(|h| h.join(".cargo")));
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if p.is_dir() && !is_unsafe_root(&p, profile.as_deref()) && !out.contains(&p) {
            out.push(p);
        }
    };
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            // Without case: `c:\users\me\.cargo\bin` on PATH would otherwise add `.cargo` (its credentials) below.
            if cargo_home
                .as_ref()
                .is_some_and(|c| fold(&d).starts_with(fold(c)))
            {
                continue; // handled below
            }
            let name = d
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            if matches!(name.as_str(), "bin" | "cmd" | "scripts" | "sbin") {
                if let Some(parent) = d.parent() {
                    push(parent.to_path_buf());
                }
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

/// Long, absolute spelling of an existing path (no 8.3 short names, no `\\?\` prefix): .NET expands short names
/// by listing parent folders, which the container does not allow.
pub fn long_path(p: &Path) -> PathBuf {
    match std::fs::canonicalize(p) {
        Ok(c) => {
            let s = c.to_string_lossy().into_owned();
            match s.strip_prefix(r"\\?\UNC\") {
                Some(rest) => PathBuf::from(format!(r"\\{rest}")),
                None => PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s).to_owned()),
            }
        }
        Err(_) => p.to_path_buf(),
    }
}

fn unicode(paths: impl IntoIterator<Item = PathBuf>) -> Result<Vec<String>, ExecError> {
    let mut v: Vec<String> = paths
        .into_iter()
        .map(|p| {
            p.into_os_string()
                .into_string()
                .map_err(|_| ExecError::Refused("MXC needs Unicode paths".into()))
        })
        .collect::<Result<_, _>>()?;
    v.sort();
    v.dedup();
    Ok(v)
}

/// The container request for one command (exposed for tests).
pub fn request(
    argv: &[String],
    cwd: &Path,
    env: &[(String, String)],
    policy: &SandboxPolicy,
) -> Result<ExecutionRequest, ExecError> {
    if argv.is_empty() {
        return Err(ExecError::Refused("empty command".into()));
    }
    let cwd = &long_path(cwd);
    let mut write: Vec<PathBuf> = policy.writable_roots.iter().map(|p| long_path(p)).collect();
    if let Some(t) = &policy.temp_dir {
        write.push(long_path(t));
    }
    if policy.mode == SandboxMode::ReadOnly {
        write.clear();
    }
    let mut read: Vec<PathBuf> = codex_derived::mxc_policy::platform_read_roots();
    read.extend(
        policy
            .read_roots
            .iter()
            .filter(|p| p.exists())
            .map(|p| long_path(p)),
    );
    if policy.mode == SandboxMode::ReadOnly {
        read.extend(policy.writable_roots.iter().map(|p| long_path(p)));
    }
    read.retain(|r| !write.iter().any(|w| w == r));
    // Node (and other runtimes) read the attributes of every folder from the drive root down to a file
    // (`fs.realpathSync`): the parents of what the command may use can be listed, never read.
    let parents: Vec<PathBuf> = if BaseContainerRunner::supports_enumerate_paths() {
        let mut out: Vec<PathBuf> = Vec::new();
        for p in write.iter().chain(read.iter()).chain(std::iter::once(cwd)) {
            for a in p.ancestors().skip(1).filter(|a| a.has_root()) {
                let a = a.to_path_buf();
                if !out.contains(&a) && !write.contains(&a) && !read.contains(&a) {
                    out.push(a);
                }
            }
        }
        out
    } else {
        Vec::new()
    };
    let mut req = ExecutionRequest {
        script_code: codex_derived::mxc_policy::command_line(argv)
            .map_err(|e| ExecError::Refused(format!("command line: {e}")))?,
        working_directory: cwd
            .to_str()
            .ok_or_else(|| ExecError::Refused("MXC needs a Unicode working directory".into()))?
            .to_owned(),
        env: Some(env.iter().map(|(k, v)| format!("{k}={v}")).collect()),
        policy: codex_derived::mxc_policy::container_policy(
            unicode(write)?,
            unicode(read)?,
            policy.network,
        ),
        ..Default::default()
    };
    req.policy.enumerate_paths = unicode(parents)?;
    // BaseContainer resolves overlapping paths itself (deny > read-only > read-write, the more specific path wins):
    // MXC's object normalizer is not called (Codex f021168; its metadata preflight refused usable grants).
    Ok(req)
}

/// Runs `argv` in a container (blocking; call from a blocking thread). On timeout the whole tree is terminated.
pub fn run(
    argv: &[String],
    cwd: &Path,
    env: &[(String, String)],
    policy: &SandboxPolicy,
    timeout: Duration,
    stop: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    live: Option<crate::Live>,
) -> Result<ExecOutput, ExecError> {
    let req = request(argv, cwd, env, policy)?;
    let start = Instant::now();
    let mut logger = Logger::new(Mode::Buffer);
    // The SDK's diagnostic buffer may contain the command or environment: only the error message is surfaced.
    let mut child = BaseContainerRunner::new()
        .spawn(&req, &mut logger, StdioMode::Pipes)
        .map_err(|e| ExecError::Spawn {
            program: argv[0].clone(),
            source: std::io::Error::other(e.error_message),
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
    // A background descendant may hold a pipe open: give it a grace period, then close the streams.
    let grace = Instant::now() + Duration::from_secs(2);
    let join = |h: Option<std::thread::JoinHandle<Vec<u8>>>,
                closer: &Option<Box<dyn mxc_sdk::mxc_common::sandbox_process::StreamCloser>>|
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
        stdout: crate::decode_output(&stdout),
        stderr: crate::decode_output(&stderr),
        timed_out,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The profile is refused however PATH spells it, and a relative PATH entry (it would resolve against the
    /// harness's working directory) is never a read root.
    #[test]
    fn unsafe_roots_ignore_case_and_relative_entries() {
        let profile = Path::new(r"C:\Users\Me");
        assert!(is_unsafe_root(Path::new(r"c:\users\me"), Some(profile)));
        assert!(is_unsafe_root(Path::new(r"C:\USERS\ME\"), Some(profile)));
        assert!(is_unsafe_root(Path::new(r"C:\"), Some(profile)));
        for rel in [".", r".\bin", r"node_modules\.bin", r"\tools", r"C:tools"] {
            assert!(is_unsafe_root(Path::new(rel), Some(profile)), "{rel}");
        }
        assert!(!is_unsafe_root(
            Path::new(r"C:\Users\Me\.cargo\bin"),
            Some(profile)
        ));
        assert!(fold(Path::new(r"c:\users\me\.CARGO\bin"))
            .starts_with(fold(Path::new(r"C:\Users\Me\.cargo"))));
    }
}
