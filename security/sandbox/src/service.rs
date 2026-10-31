//! Long-running processes inside the sandbox with live stdio: a vendor agent CLI driven by the CliBackend runs in its
//! own YOLO mode while Agenium's sandbox decides what it can touch (ADR-56). Same backends and policy shape as
//! `Executor::run` (MXC on Windows, bubblewrap on Linux, Seatbelt on macOS), but the program is started directly
//! (no shell) and stdin/stdout/stderr stay open for the caller.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use crate::{ExecError, SandboxPolicy};

#[cfg(windows)]
use mxc_sdk::mxc_common::sandbox_process::SandboxProcess;

#[cfg(windows)]
type Child = Box<dyn SandboxProcess>;
#[cfg(any(target_os = "linux", target_os = "macos"))]
type Child = mxc_sdk::v1::MxcProcess;

/// The process; dropping it kills the whole contained tree.
pub struct Contained {
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    child: Child,
    /// The token every process of the tree carries (`unix::STRAY_TOKEN_ENV`): what is left when it ends is found by it.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    token: String,
}

pub struct ContainedStdio {
    pub stdin: Option<Box<dyn Write + Send>>,
    pub stdout: Option<Box<dyn Read + Send>>,
    pub stderr: Option<Box<dyn Read + Send>>,
}

impl Contained {
    /// Exit code once the process has ended.
    pub fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        return self.child.try_wait();
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        Ok(Some(-1))
    }

    /// Terminates the process and everything it started.
    pub fn kill(&mut self) {
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        {
            let _ = self.child.kill_for_timeout();
            let _ = self.child.wait();
        }
    }
}

impl Drop for Contained {
    fn drop(&mut self) {
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        if matches!(self.child.try_wait(), Ok(None)) {
            self.kill();
        }
        // On macOS nothing else ends what the CLI left running (a server it started with `&`, a daemon): no PID
        // namespace (Linux), no job object (Windows), and the kill above is a no-op once the CLI has exited itself
        // (audit L2, 2026-10-31). Its processes carry the tree's token.
        #[cfg(target_os = "macos")]
        crate::unix::kill_strays(&self.token);
    }
}

/// POSIX shell quoting of one argument.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn quote(a: &str) -> String {
    format!("'{}'", a.replace('\'', r"'\''"))
}

/// Starts `argv` (program and arguments) in `cwd` under `policy` with the clean environment plus `env`. Fails when
/// this OS has no working sandbox backend (fail-closed: the caller must not fall back to running it uncontained).
pub fn spawn_contained(
    argv: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    policy: &SandboxPolicy,
) -> Result<(Contained, ContainedStdio), ExecError> {
    if argv.is_empty() {
        return Err(ExecError::Refused("empty command".into()));
    }
    let backend = crate::detect().backend;
    #[cfg(windows)]
    {
        use mxc_sdk::mxc_common::logger::{Logger, Mode};
        use mxc_sdk::mxc_common::sandbox_process::{SandboxBackend, StdioMode};
        use mxc_sdk::process_container_common::base_container_runner::BaseContainerRunner;
        if backend != Some(crate::Backend::WindowsMxc) {
            return Err(ExecError::Refused(
                "no sandbox backend works on this machine".into(),
            ));
        }
        let env = crate::mxc_env(env, policy)?;
        let req = crate::mxc::request(argv, cwd, &env, policy)?;
        let mut logger = Logger::new(Mode::Buffer);
        let mut child = BaseContainerRunner::new()
            .spawn(&req, &mut logger, StdioMode::Pipes)
            .map_err(|e| ExecError::Spawn {
                program: argv[0].clone(),
                source: std::io::Error::other(e.error_message),
            })?;
        let stdio = ContainedStdio {
            stdin: child.take_stdin(),
            stdout: child.take_stdout(),
            stderr: child.take_stderr(),
        };
        Ok((Contained { child }, stdio))
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        if !matches!(
            backend,
            Some(crate::Backend::Bubblewrap | crate::Backend::Seatbelt)
        ) {
            return Err(ExecError::Refused(
                "no sandbox backend works on this machine".into(),
            ));
        }
        // A contained service (a CLI agent) is useless without the network it asked for: say so instead of starting
        // it without (commands fall back; see unix::effective).
        if policy.network {
            if let Err(why) = crate::unix::private_network() {
                return Err(ExecError::Refused(format!(
                    "{} needs the network inside the sandbox, but {why}",
                    argv[0]
                )));
            }
        }
        let mut env = crate::clean_env(env);
        if let Some(t) = &policy.temp_dir {
            crate::prepare_temp_dir(policy)?;
            env.insert("TMPDIR".into(), t.display().to_string());
        }
        let token = crate::unix::stray_token();
        env.insert(crate::unix::STRAY_TOKEN_ENV.into(), token.clone());
        let env: Vec<(String, String)> = env.into_iter().collect();
        let line: Vec<String> = argv.iter().map(|a| quote(a)).collect();
        let mut child = mxc_sdk::v1::spawn(
            crate::unix::request(&line.join(" "), cwd, &env, policy),
            Default::default(),
        )
        .map_err(|e| ExecError::Spawn {
            program: argv[0].clone(),
            source: std::io::Error::other(e.to_string()),
        })?;
        let stdio = ContainedStdio {
            stdin: child.take_stdin(),
            stdout: child.take_stdout(),
            stderr: child.take_stderr(),
        };
        Ok((Contained { child, token }, stdio))
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = (cwd, env, policy, backend);
        Err(ExecError::Refused("no sandbox backend on this OS".into()))
    }
}
