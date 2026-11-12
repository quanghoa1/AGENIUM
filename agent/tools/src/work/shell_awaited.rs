//! `shell` in a vendor CLI's run (ADR-133): the command runs as a job the run waits for. Ended within the call's limit,
//! it answers as a plain `shell` call; still going (running, or waiting for the user's approval), the call answers that
//! it goes on, and its end reaches the agent as a message once its turn is over (`ended_message`).

use super::*;
use crate::jobs::{Ended, DENIED, STOPPED};

pub(super) async fn call(
    ctx: &ToolCtx,
    spec: agenium_sandbox::CommandSpec,
    limit: Duration,
    wanted: Option<String>,
) -> Result<ToolOutput, ToolError> {
    let timeout = spec.timeout;
    let id = ctx.jobs.start_awaited(
        &ctx.executor,
        &ctx.policy,
        spec,
        ctx.gate.clone(),
        ctx.stop.clone(),
        wanted,
    );
    if ctx.jobs.wait_end(id, limit).await {
        if let Some(r) = ctx.jobs.take_ended(id) {
            return answer(ctx, r, timeout);
        }
    }
    let state = if ctx.jobs.is_running(id) {
        "still running"
    } else {
        "still waiting for the user's approval"
    };
    Ok(ToolOutput::text(format!(
        "The command is {state} after {} s: it goes on as job {id} (time limit {} s). Its result comes to you as a \
         message when your turn is over and it has ended: end your turn now unless you have other work meanwhile. \
         Do not start it again. shell_output {{\"job\": {id}}} shows what it printed so far, shell_stop ends it.",
        limit.as_secs(),
        timeout.as_secs()
    )))
}

fn answer(
    ctx: &ToolCtx,
    r: Result<agenium_sandbox::ExecOutput, String>,
    timeout: Duration,
) -> Result<ToolOutput, ToolError> {
    match r {
        Ok(out) => Ok(ctx.finish_output(command_text(&out, timeout, ctx.stopped()), false)),
        Err(e) if e.starts_with(DENIED) => Err(denied(&e[DENIED.len()..])),
        Err(e) if e == STOPPED => Err(ToolError::new(
            "stopped",
            "the command was stopped before it ended",
        )),
        Err(e) => Err(ToolError::new("exec_failed", e)),
    }
}

/// The message that hands an awaited command's end to the agent.
pub fn ended_message(ctx: &ToolCtx, e: &Ended) -> String {
    let how = match &e.result {
        Ok(out) => {
            ctx.finish_output(command_text(out, e.timeout, false), false)
                .text
        }
        Err(r) if r.starts_with(DENIED) => format!(
            "it was not run: {}. If you need it, explain why with ask_question or request_permission \
             (kind=command).",
            &r[DENIED.len()..]
        ),
        Err(r) if r == STOPPED => "it was stopped before it ended.".into(),
        Err(r) => format!("it could not run: {r}"),
    };
    format!(
        "Your command `{}` (job {}) has ended: {how}",
        e.command, e.id
    )
}
