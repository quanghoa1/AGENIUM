//! The row of a run of the Lead or the planner (a chat turn, an inbox run, a plan, a review): inserted with its
//! `RunStarted` event, the step the four kinds of run share.

use super::*;

impl Runtime {
    /// Opens run `id` of `agent` (on `ticket` for a review): from now on it counts as going on (`runs::unfinished`).
    pub(super) fn open_run(
        &self,
        agent: &agenium_store::core::agents::Agent,
        ticket: Option<TicketId>,
        id: RunId,
    ) -> Result<RunId> {
        Ok(self
            .app
            .store()
            .commit(None, "system", |w| -> Result<RunId> {
                runs::insert(
                    w.tx,
                    &runs::Run {
                        id,
                        ticket,
                        agent: agent.id,
                        provider: agent.model.provider.clone(),
                        model: agent.model.model.clone(),
                        started_at: now_iso(),
                        ended_at: None,
                        outcome: None,
                        steps: 0,
                        usage: UsageSummary::default(),
                        heartbeat: Some(now_iso()),
                    },
                )?;
                w.emit(DomainEvent::RunStarted {
                    run: id,
                    agent: agent.id,
                    ticket,
                })?;
                Ok(id)
            })?
            .value)
    }
}
