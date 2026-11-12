//! The real pre-merge check (M5): runs the project's `[checks] pre_merge` command (set by the team template or by
//! hand) in the ticket worktree before the merge queue fast-forwards. It runs inside Agenium's sandbox when a
//! backend works (writes stay in the worktree, the network as `[sandbox] network` gives the work's own commands: a
//! suite that starts its own server reaches it); without a backend it is refused (fail-closed), because
//! it is a shell command and Agenium does not run shell commands unsandboxed on its own. A non-zero exit fails the
//! merge and the ticket is reopened with the output, which the merge flow already handles.
//!
//! The headless CLI asks at the terminal first (`apps/cli`), then runs the command with [`run_check`]; this one runs
//! automatically and is what the desktop app and non-interactive runtimes use.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use agenium_sandbox::{CommandSpec, Executor, SandboxPolicy, Shell};
use agenium_store::core::projects;
use agenium_vcs::{CheckOutcome, CheckRunner, NoCheck};

use crate::app::CheckFactory;
use crate::manifest::ProjectManifest;

/// Pre-merge check command timeout.
const TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Builds a [`SandboxCheck`] from the project's `pre_merge` command, or a no-op when it is empty.
pub struct SandboxCheckFactory;

impl CheckFactory for SandboxCheckFactory {
    fn for_project(
        &self,
        _: &projects::Project,
        manifest: &ProjectManifest,
        network: bool,
    ) -> Arc<dyn CheckRunner> {
        let cmd = manifest.checks.pre_merge.trim().to_owned();
        if cmd.is_empty() {
            Arc::new(NoCheck)
        } else {
            Arc::new(SandboxCheck { cmd, network })
        }
    }
}

struct SandboxCheck {
    cmd: String,
    network: bool,
}

impl CheckRunner for SandboxCheck {
    fn run(&self, worktree: &Path) -> CheckOutcome {
        // With the sandbox turned off (ADR-160) the check runs directly on the host, like every other command.
        if agenium_sandbox::detect().backend.is_none() && !agenium_sandbox::is_off() {
            return CheckOutcome {
                ok: false,
                log: format!(
                    "pre-merge check `{}` needs Agenium's sandbox and no backend works on this machine",
                    self.cmd
                ),
            };
        }
        run_check(&self.cmd, worktree, self.network)
    }
}

/// Runs a pre-merge check command in `worktree` under the workspace policy (with the network when `network`) and
/// turns the exit into an outcome. Safe to call from inside a Tokio runtime (it runs on a scratch thread then).
pub fn run_check(cmd: &str, worktree: &Path, network: bool) -> CheckOutcome {
    let ex = Executor::new(Shell::detect());
    let spec = CommandSpec {
        command: cmd.to_owned(),
        cwd: worktree.to_path_buf(),
        timeout: TIMEOUT,
        ..Default::default()
    };
    let mut policy = SandboxPolicy::workspace(worktree);
    policy.network = network;
    // Run on a scratch thread so a current Tokio runtime is not blocked.
    let r = match tokio::runtime::Handle::try_current() {
        Ok(h) => std::thread::scope(|s| {
            s.spawn(|| h.block_on(ex.run(&spec, &policy)))
                .join()
                .unwrap()
        }),
        Err(_) => tokio::runtime::Runtime::new()
            .map_err(agenium_sandbox::ExecError::Io)
            .and_then(|rt| rt.block_on(ex.run(&spec, &policy))),
    };
    match r {
        Ok(o) => CheckOutcome {
            ok: o.exit_code == Some(0) && !o.timed_out,
            log: format!("$ {cmd}\n{}{}", o.stdout, o.stderr),
        },
        Err(e) => CheckOutcome {
            ok: false,
            log: format!("pre-merge check failed to run: {e}"),
        },
    }
}
