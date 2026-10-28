//! `shell_output` and `shell_stop`: the background commands `shell` started (ADR-129).

use super::*;
use crate::jobs::{JobRead, JobState};

/// The longest `shell_output` waits for new output.
const MAX_WAIT_MS: u64 = 30_000;

/// Read what a background command printed since your last read, and whether it is still running.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellOutputArgs {
    /// The id `shell` returned when it started the command in the background.
    pub job: u32,
    /// Wait up to this long (milliseconds, at most 30000) for new output or the end; default 0.
    #[serde(default)]
    pub wait_ms: Option<u64>,
}

/// Stop a background command (its whole process tree).
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShellStopArgs {
    /// The id `shell` returned when it started the command in the background.
    pub job: u32,
}

fn render(ctx: &ToolCtx, id: u32, r: JobRead) -> ToolOutput {
    let state = match r.state {
        JobState::Running { secs } => format!("still running ({secs} s)"),
        JobState::Exited {
            code: Some(c),
            timed_out: false,
        } => format!("ended, exit code {c}"),
        JobState::Exited {
            timed_out: true, ..
        } => "ended: it ran for its whole time limit (process tree killed)".into(),
        JobState::Exited { code: None, .. } => "ended, terminated by a signal".into(),
        JobState::Stopped => "stopped (process tree killed)".into(),
        JobState::Failed(e) => format!("could not run: {e}"),
    };
    let mut text = format!("background command {id} `{}`: {state}\n", r.command);
    if r.new_output.is_empty() {
        text.push_str("(no new output)\n");
    } else {
        text.push_str("--- new output ---\n");
        text.push_str(&r.new_output);
    }
    ctx.finish_output(text, false)
}

pub struct ShellOutput;

#[async_trait]
impl Tool for ShellOutput {
    fn name(&self) -> &'static str {
        "shell_output"
    }
    fn description(&self) -> &'static str {
        "Read what a background command (started by shell with background=true) printed since your last read, and \
whether it is still running. wait_ms waits for new output, e.g. until a server says it is listening."
    }
    fn parameters(&self) -> serde_json::Value {
        args_schema::<ShellOutputArgs>()
    }
    fn read_only(&self) -> bool {
        true
    }
    async fn call(&self, ctx: &ToolCtx, a: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let a: ShellOutputArgs = args(a)?;
        let wait = Duration::from_millis(a.wait_ms.unwrap_or(0).min(MAX_WAIT_MS));
        let r = ctx
            .jobs
            .read(a.job, wait)
            .await
            .map_err(|e| ToolError::new("unknown_job", e))?;
        Ok(render(ctx, a.job, r))
    }
}

pub struct ShellStop;

#[async_trait]
impl Tool for ShellStop {
    fn name(&self) -> &'static str {
        "shell_stop"
    }
    fn description(&self) -> &'static str {
        "Stop a background command started by shell with background=true (its whole process tree) and read its last \
output. Background commands also stop when your run ends."
    }
    fn parameters(&self) -> serde_json::Value {
        args_schema::<ShellStopArgs>()
    }
    async fn call(&self, ctx: &ToolCtx, a: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let a: ShellStopArgs = args(a)?;
        let r = ctx
            .jobs
            .stop(a.job)
            .await
            .map_err(|e| ToolError::new("unknown_job", e))?;
        Ok(render(ctx, a.job, r))
    }
}
