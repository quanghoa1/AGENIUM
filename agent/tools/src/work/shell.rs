//! `shell`: a command run in the sandbox.

use super::*;

/// Run a shell command in your worktree.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellArgs {
    pub command: String,
    /// Working directory relative to the worktree root.
    #[serde(default)]
    pub workdir: Option<String>,
    /// Timeout in milliseconds (default 120000, at most 600000).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Start it in the background and return at once with a job id (a server, a watcher, a long build); read its
    /// output with shell_output, end it with shell_stop. It ends with your run at the latest.
    #[serde(default)]
    pub background: bool,
    /// Only for a command that cannot work in the sandbox (it needs the user's credentials, docker, a server on
    /// localhost, a file outside your worktree): why. The user is asked (or has chosen to allow these) before it runs
    /// outside the sandbox, directly on their computer. A command the sandbox blocks goes to the user by itself.
    #[serde(default)]
    pub outside_sandbox: Option<String>,
}

pub struct ShellTool;

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &'static str {
        "shell"
    }
    fn description(&self) -> &'static str {
        "Run a shell command in your worktree: builds, tests, linters, git status, package installs inside the \
project. Returns the exit code, stdout and stderr; long outputs are cut and the full output is saved as an artifact. \
Read, search and change files with the file tools rather than cat, sed or echo; start a server or a watcher with \
background and read it with shell_output. Commands run in a sandbox: one it blocks is offered to the user to \
run outside it, and outside_sandbox asks that up front for a command that cannot work inside."
    }
    fn parameters(&self) -> serde_json::Value {
        args_schema::<ShellArgs>()
    }
    async fn call(&self, ctx: &ToolCtx, a: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let a: ShellArgs = args(a)?;
        if a.command.trim().is_empty() {
            return Err(ToolError::new("invalid_arguments", "command is empty"));
        }
        let cwd = ctx.resolve(a.workdir.as_deref().unwrap_or("."), false)?;
        if !paths::within(&cwd, &ctx.root) {
            return Err(ToolError::new(
                "out_of_scope",
                "workdir must be inside your worktree",
            ));
        }
        let timeout = a
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(ctx.limits.shell_timeout)
            .min(ctx.limits.shell_timeout_max);
        let mut spec = agenium_sandbox::CommandSpec {
            command: a.command.clone(),
            cwd: cwd.clone(),
            timeout,
            env: ctx.env.clone(),
            ..Default::default()
        };
        let wanted = a
            .outside_sandbox
            .as_deref()
            .map(str::trim)
            .filter(|w| !w.is_empty());
        // A vendor CLI waits only so long for a tool: its command goes on as a job the run waits for (ADR-133).
        if let (Some(limit), false) = (ctx.foreground_limit, a.background) {
            return super::shell_awaited::call(ctx, spec, limit, wanted.map(str::to_owned)).await;
        }
        // An approval the user has not given yet is given up when the run is stopped.
        let decision = tokio::select! {
            d = ctx.gate.check(&a.command, &cwd) => d,
            _ = ctx.until_stopped() => return Err(ToolError::new("stopped", "the run was stopped before the command was approved")),
        };
        if let GateDecision::Deny(reason) = decision {
            return Err(denied(&reason));
        }
        if a.background {
            // A background command leaves the sandbox only when asked up front (ADR-161).
            let mut policy = ctx.policy.clone();
            if let (Some(why), true) = (wanted, escalate::sandboxed(&ctx.policy)) {
                let why = format!("the agent asks to run it outside the sandbox: {why}");
                spec.stop = Some(ctx.stop.clone());
                match escalate::ask(&*ctx.gate, &spec, &why).await {
                    Some(GateDecision::Allow) => policy = escalate::outside(&ctx.policy),
                    Some(GateDecision::Deny(r)) => {
                        return Err(denied(&format!(
                            "running it outside the sandbox was not allowed: {r}"
                        )))
                    }
                    None => {
                        return Err(ToolError::new(
                            "stopped",
                            "the run was stopped before the command was approved",
                        ))
                    }
                }
                spec.stop = None;
            }
            let id = ctx
                .jobs
                .start(&ctx.executor, &policy, spec)
                .map_err(|e| ToolError::new("too_many_jobs", e))?;
            return Ok(ToolOutput::text(format!(
                "started in the background as job {id}: read its output with shell_output {{\"job\": {id}, \
                 \"wait_ms\": 5000}}, end it with shell_stop {{\"job\": {id}}}"
            )));
        }
        // The run's stop ends the command at once, its whole process tree with it.
        spec.stop = Some(ctx.stop.clone());
        // Sandbox first; a command it blocks goes to the user to run outside (ADR-161).
        let out = match escalate::run(&ctx.executor, &ctx.policy, &spec, &*ctx.gate, wanted).await {
            Ok(ran) => ran.into_output(),
            Err(e) if e.starts_with(DENIED) => return Err(denied(&e[DENIED.len()..])),
            Err(e) if e == STOPPED => {
                return Err(ToolError::new(
                    "stopped",
                    "the run was stopped before the command was approved",
                ))
            }
            Err(e) => return Err(ToolError::new("exec_failed", e)),
        };
        Ok(ctx.finish_output(command_text(&out, timeout, ctx.stopped()), false))
    }
}

/// The refusal of a command the gate did not allow.
pub(super) fn denied(reason: &str) -> ToolError {
    ToolError::new(
        "command_denied",
        format!("{reason}. If you need it, explain why with ask_question or request_permission (kind=command)."),
    )
}

/// What a command that ended says: how it ended, then its output.
pub(super) fn command_text(
    out: &agenium_sandbox::ExecOutput,
    timeout: Duration,
    stopped: bool,
) -> String {
    let mut text = match (out.timed_out, out.exit_code) {
        (false, None) if stopped => {
            "stopped: the run was stopped while the command ran (process tree killed)\n".into()
        }
        (true, _) => format!(
            "timed out after {} ms (process tree killed)\n",
            timeout.as_millis()
        ),
        (false, Some(c)) => format!("exit code {c}\n"),
        (false, None) => "terminated by a signal\n".into(),
    };
    if !out.stdout.is_empty() {
        text.push_str("--- stdout ---\n");
        text.push_str(&out.stdout);
        if !out.stdout.ends_with('\n') {
            text.push('\n');
        }
    }
    if !out.stderr.is_empty() {
        text.push_str("--- stderr ---\n");
        text.push_str(&out.stderr);
    }
    text
}

use crate::jobs::{escalate, DENIED, STOPPED};
use crate::paths;
