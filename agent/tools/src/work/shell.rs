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
}

pub struct ShellTool;

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &'static str {
        "shell"
    }
    fn description(&self) -> &'static str {
        "Run a shell command in your worktree (no network unless granted). Returns exit code, stdout and stderr. \
Long outputs are cut; the full output is saved as an artifact."
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
        if let GateDecision::Deny(reason) = ctx.gate.check(&a.command, &cwd).await {
            return Err(ToolError::new("command_denied", format!("{reason}. If you need it, explain why with ask_question or request_permission (kind=command).")));
        }
        let timeout = a
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(ctx.limits.shell_timeout)
            .min(ctx.limits.shell_timeout_max);
        let spec = agenium_sandbox::CommandSpec {
            command: a.command.clone(),
            cwd,
            timeout,
            env: ctx.env.clone(),
        };
        let out = ctx
            .executor
            .run(&spec, &ctx.policy)
            .await
            .map_err(|e| ToolError::new("exec_failed", e.to_string()))?;
        let mut text = match (out.timed_out, out.exit_code) {
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
        Ok(ctx.finish_output(text, false))
    }
}

use crate::paths;
