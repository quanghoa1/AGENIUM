//! What a `submit_report` waits for, in an API run and a CLI's run alike: a command of the run still going (its result
//! is not known yet, ADR-133) and the agent's own checklist with items not done (ADR-164). The checklist refuses at
//! most [`TODO_REFUSALS`] times in a run; after that the report goes through, the refusals kept in the run's log.

use std::sync::atomic::Ordering;

use crate::{TodoStatus, ToolCtx, ToolError};

/// Refusals for an unfinished checklist in one run before the report goes through.
pub const TODO_REFUSALS: u32 = 2;

/// Why `submit_report` is refused now, if it is.
pub fn submit_refusal(ctx: &ToolCtx) -> Option<ToolError> {
    let going = ctx.jobs.awaited_going();
    if !going.is_empty() {
        return Some(ToolError::new(
            "commands_running",
            format!(
                "{} still going: end your turn and wait for {} (it comes as a message), then report",
                going
                    .iter()
                    .map(|(id, c)| format!("job {id} `{c}`"))
                    .collect::<Vec<_>>()
                    .join(", "),
                if going.len() == 1 {
                    "its result"
                } else {
                    "their results"
                }
            ),
        ));
    }
    let open: Vec<String> = ctx
        .todo()
        .into_iter()
        .filter(|i| i.status != TodoStatus::Done)
        .map(|i| format!("- {}", i.text))
        .collect();
    if open.is_empty() {
        return None;
    }
    let n = ctx.todo_refusals.fetch_add(1, Ordering::Relaxed) + 1;
    (n <= TODO_REFUSALS).then(|| {
        ToolError::new(
            "unfinished_todo",
            format!(
                "Your checklist still has {} item(s) not done:\n{}\nFinish them, or update the checklist with todo \
                 (mark done what is done; drop what is no longer needed and say why in the report), then call \
                 submit_report again (refusal {n} of {TODO_REFUSALS}; after that the report goes through as it is).",
                open.len(),
                open.join("\n")
            ),
        )
    })
}
