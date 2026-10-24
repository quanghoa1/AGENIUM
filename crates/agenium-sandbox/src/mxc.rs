//! Windows backend: Microsoft MXC process containers through `mxc-sdk` (MIT), the native path Codex's
//! `codex-mxc-sandbox` uses (S2, ADR-53). No Codex binary, no administrator rights, no extra users.
//!
//! Portions adapted from OpenAI Codex `codex-rs/mxc-sandbox` (Apache-2.0, https://github.com/openai/codex): the
//! availability probe and the shape of the container policy (platform read roots, registry read capability,
//! desktop UI isolation, deny-by-default network). See NOTICE.
//!
//! Policy: the writable roots (ticket worktree, per-run temp dir) are read-write; the Windows platform folders,
//! toolchain roots and explicit read roots (repository `.git`, skill folders) are read-only; everything else is
//! invisible; network egress, ingress and host loopback are denied unless the policy allows the network.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use flatbuffers::FlatBufferBuilder;
use mxc_sdk::mxc_common::cmdline::{cmdline_from_argv_for_context, CommandLineContext};
use mxc_sdk::mxc_common::filesystem_object::normalize_object_conflicts;
use mxc_sdk::mxc_common::logger::{Logger, Mode};
use mxc_sdk::mxc_common::models::{
    BaseProcessUiConfig, ContainerPolicy, ExecutionRequest, FallbackPolicy, NetworkAction,
    NetworkEgressPolicy, NetworkIngressPolicy, UiPolicy,
};
use mxc_sdk::mxc_common::sandbox_process::{SandboxBackend, StdioMode};
use mxc_sdk::mxc_common::ui_policy::resolve_ui_restrictions;
use mxc_sdk::process_container_common::base_container_runner::BaseContainerRunner;
use mxc_sdk::process_container_common::job_object::to_job_object_uilimit_mask;
use mxc_sdk::process_container_common::{
    ProcessSecurityEnvironmentApi, SecurityEnvironmentStartupInfo,
    PROCESS_SECURITY_ENVIRONMENT_FLAG_NONE,
};
use mxc_sdk::process_security_environment_spec::process_security_environment_layout::{
    finish_process_security_environment_buffer, EndpointPolicyT, FilterAction, NetworkPolicyT,
    ProcessSecurityEnvironmentT, SchemaVersionT,
};

use crate::{ExecError, ExecOutput, SandboxMode, SandboxPolicy};

// tracelogging (pulled in by mxc-sdk) leaves the ETW import library to the application on GNU targets.
#[link(name = "advapi32")]
unsafe extern "system" {}

/// Whether a native MXC process security environment can be created here (probe without launching anything;
/// cached). Older AppContainer fallbacks of MXC do not count.
pub fn available() -> bool {
    static USABLE: OnceLock<bool> = OnceLock::new();
    *USABLE.get_or_init(|| {
        let policy = ContainerPolicy::default();
        let mut egress = EndpointPolicyT::default();
        egress.default_action = FilterAction::deny;
        let mut network = NetworkPolicyT::default();
        network.egress = Some(Box::new(egress));
        let mut spec = ProcessSecurityEnvironmentT::default();
        spec.version = SchemaVersionT { major: 1, minor: 0 };
        spec.disallow_win32k_system_calls = policy.ui.disable;
        spec.ui_restrictions = u64::from(to_job_object_uilimit_mask(&resolve_ui_restrictions(
            &policy.ui,
            &policy.base_process_ui,
        )));
        spec.network_policy = Some(Box::new(network));
        let mut builder = FlatBufferBuilder::with_capacity(1024);
        let packed = spec.pack(&mut builder);
        finish_process_security_environment_buffer(&mut builder, packed);
        ProcessSecurityEnvironmentApi::load()
            .and_then(|api| {
                api.create(
                    builder.finished_data(),
                    PROCESS_SECURITY_ENVIRONMENT_FLAG_NONE,
                )
            })
            .and_then(|env| {
                let info = SecurityEnvironmentStartupInfo::new(Default::default(), env.raw(), &[]);
                env.close();
                info.map(drop)
            })
            .is_ok()
    })
}

/// Windows folders every process needs to start (Windows, Program Files, ProgramData).
pub fn platform_read_roots() -> Vec<PathBuf> {
    [
        "SystemRoot",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
    ]
    .iter()
    .filter_map(std::env::var_os)
    .map(PathBuf::from)
    .filter(|p| p.is_dir())
    .collect()
}

fn is_unsafe_root(p: &Path, profile: Option<&Path>) -> bool {
    p.parent().is_none() || profile.is_some_and(|h| p == h) || p.components().count() <= 1
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
            if cargo_home.as_ref().is_some_and(|c| d.starts_with(c)) {
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
    let mut read: Vec<PathBuf> = platform_read_roots();
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
    let net = if policy.network {
        NetworkAction::Allow
    } else {
        NetworkAction::Deny
    };
    let mut req = ExecutionRequest {
        script_code: cmdline_from_argv_for_context(argv, CommandLineContext::WindowsCreateProcess)
            .map_err(|e| ExecError::Refused(format!("command line: {e}")))?,
        working_directory: cwd
            .to_str()
            .ok_or_else(|| ExecError::Refused("MXC needs a Unicode working directory".into()))?
            .to_owned(),
        env: Some(env.iter().map(|(k, v)| format!("{k}={v}")).collect()),
        policy: ContainerPolicy {
            capabilities: vec!["registryRead".to_owned()],
            readwrite_paths: unicode(write)?,
            readonly_paths: unicode(read)?,
            denied_paths: vec![],
            fallback: FallbackPolicy {
                allow_dacl_mutation: false,
            },
            network_egress: Some(NetworkEgressPolicy {
                default: net,
                ..Default::default()
            }),
            network_ingress: Some(NetworkIngressPolicy {
                default: net,
                host_loopback: net,
            }),
            network_specified: true,
            network_mode_specified: true,
            // PowerShell and some toolchains need Win32k and desktop handles at startup; clipboard, input
            // injection and system-control restrictions stay.
            ui: UiPolicy {
                disable: false,
                ..Default::default()
            },
            base_process_ui: BaseProcessUiConfig {
                isolation: "desktop".to_owned(),
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    let mut logger = Logger::new(Mode::Buffer);
    if let Some(n) = normalize_object_conflicts(&req.policy, &mut logger)
        .map_err(|e| ExecError::Refused(format!("MXC policy: {e}")))?
    {
        req.policy = n;
    }
    Ok(req)
}

fn reader(mut r: Box<dyn Read + Send>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = r.read_to_end(&mut b);
        b
    })
}

/// Runs `argv` in a container (blocking; call from a blocking thread). On timeout the whole tree is terminated.
pub fn run(
    argv: &[String],
    cwd: &Path,
    env: &[(String, String)],
    policy: &SandboxPolicy,
    timeout: Duration,
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
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        timed_out,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}
