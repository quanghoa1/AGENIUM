//! Tasks whose agent must ask the user before it goes on (`ask-first`): the user's answer comes from here, and the
//! ticket's next run must go on from the first. A vendor CLI that keeps its session resumes it (ADR-134): its run log
//! says so, and a resume the CLI refused (it started again from the conversation as text) fails the task. The code word
//! is not in the question or the answer: a resumed run knows it from its session, or reads the file again.

use agenium_core::*;
use agenium_orchestrator::app::App;
use agenium_store::core::{comments, runs};

/// The user's answer to each task's question.
const ANSWERS: [(&str, &str); 1] = [(
    "ask-first",
    "Write it to deliverables/word.txt, nothing else in the file.",
)];

/// Answers the ticket's open blocking question when the task has an answer for it; whether it did.
pub fn answer(app: &App, project: ProjectId, key: &TicketKey, task: &str) -> anyhow::Result<bool> {
    let Some((_, a)) = ANSWERS.iter().find(|(id, _)| *id == task) else {
        return Ok(false);
    };
    let t = app.ticket(project, key)?;
    let Some(q) = app
        .store()
        .read(|c| comments::open_blocking(c, t.id))?
        .into_iter()
        .next()
    else {
        return Ok(false);
    };
    app.user_answer(project, key, q.id, a, None)?;
    Ok(true)
}

/// How the ticket's CLI session went on across its runs: `Some(true)` when a run resumed it, `Some(false)` when a
/// resume was refused, `None` when no run tried (an API model, or a CLI that does not keep its session).
pub fn resumed(app: &App, ticket: TicketId) -> anyhow::Result<Option<bool>> {
    let mut seen = None;
    for r in app.store().read(|c| runs::for_ticket(c, ticket))? {
        for e in app.store().read(|c| runs::log_entries(c, r.id))? {
            if e.kind == "control" && e.data["what"] == "resume" {
                let refused = e.data["text"]
                    .as_str()
                    .is_some_and(|t| t.contains("could not be resumed"));
                if refused {
                    return Ok(Some(false));
                }
                seen = Some(true);
            }
        }
    }
    Ok(seen)
}
