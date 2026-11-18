//! Commands that outlast their `shell` call in an API run (ADR-164), as in a CLI's run (ADR-133): the run hands the
//! end of one to the agent as a message before its next step, and waits for those still going when the agent ends
//! its turn. A command keeps its own time limit; the user's stop ends it and the wait.

use super::*;

/// How often the wait looks at the run's commands.
const POLL: std::time::Duration = std::time::Duration::from_millis(250);

impl Run<'_> {
    /// The ends of the run's commands that ended since the agent was last told, as one message.
    pub(super) fn ended_commands(&self) -> Option<String> {
        let ended = self.ctx.jobs.take_ended_untold();
        (!ended.is_empty()).then(|| {
            ended
                .iter()
                .map(|e| agenium_tools::ended_message(self.ctx, e))
                .collect::<Vec<_>>()
                .join("\n\n")
        })
    }

    /// Waits until no command of the run goes on; `false` when the run was stopped meanwhile.
    pub(super) async fn wait_for_commands(&self) -> bool {
        let going = self.ctx.jobs.awaited_going();
        let what = going
            .iter()
            .map(|(id, c)| format!("job {id} `{c}`"))
            .collect::<Vec<_>>()
            .join(", ");
        self.observer.control("waiting", &what);
        while !self.ctx.jobs.awaited_going().is_empty() {
            if self.cancel.is_cancelled() {
                return false;
            }
            tokio::time::sleep(POLL).await;
        }
        true
    }
}
