//! Commands that outlast their `shell` call in an API run (ADR-164), as in a CLI's run (ADR-133): the run hands the
//! end of one to the agent as a message before its next step, and waits for those still going when the agent ends
//! its turn. A command keeps its own time limit; the user's stop ends it and the wait.

use super::*;

impl Run<'_> {
    /// The ends of the run's commands that ended since the agent was last told, as one message.
    pub(super) fn ended_commands(&self) -> Option<String> {
        agenium_tools::ended_commands(self.ctx)
    }

    /// Waits until no command of the run goes on; `false` when the run was stopped meanwhile.
    pub(super) async fn wait_for_commands(&self) -> bool {
        agenium_tools::wait_for_commands(
            self.ctx,
            || self.cancel.is_cancelled(),
            None,
            |what| self.observer.control("waiting", what),
        )
        .await
        .is_ok()
    }
}
