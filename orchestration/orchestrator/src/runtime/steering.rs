//! Which runs of a Lead are going on (ADR-104): while one is, a message to the Lead waits in the project's queue for
//! its next chat turn (ADR-149), and a chat turn starts only when none is.

use super::*;

/// One run of a Lead going on, with its loop's message queue (left empty: ADR-149).
pub(super) struct SteerQueue {
    run: RunId,
    pub(super) q: agenium_backend::Steer,
}

/// Keeps a Lead's run registered while it lasts.
pub(super) struct SteerGuard<'a> {
    rt: &'a Runtime,
    project: ProjectId,
    agent: AgentId,
    run: RunId,
}

impl<'a> SteerGuard<'a> {
    fn register(
        rt: &'a Runtime,
        project: ProjectId,
        agent: AgentId,
        run: RunId,
    ) -> (Self, agenium_backend::Steer) {
        let q = agenium_backend::Steer::default();
        rt.steering
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(agent)
            .or_default()
            .push(SteerQueue { run, q: q.clone() });
        (
            SteerGuard {
                rt,
                project,
                agent,
                run,
            },
            q,
        )
    }
}

impl Drop for SteerGuard<'_> {
    fn drop(&mut self) {
        // Only this run is removed (another run of the Lead keeps its own registration).
        let mut map = self.rt.steering.lock().unwrap_or_else(|e| e.into_inner());
        let Some(qs) = map.get_mut(&self.agent) else {
            return;
        };
        let Some(i) = qs.iter().position(|s| s.run == self.run) else {
            return;
        };
        let mine = qs.remove(i);
        if qs.is_empty() {
            map.remove(&self.agent);
        }
        drop(map);
        // Nothing is put in a run's own queue any more (ADR-149); what a vendor's loop left there waits in the queue.
        let left = std::mem::take(&mut *mine.q.lock().unwrap_or_else(|e| e.into_inner()));
        for text in left {
            if let Err(e) = self.rt.app.post_lead_message(self.project, &text) {
                tracing::warn!(error = %e, "a message left in a run's queue was not kept");
            }
        }
    }
}

impl Runtime {
    /// Registers `run` as a run of the project's Lead (a Solo agent is its own team's Lead) going on: a message to the
    /// Lead waits for it to end (ADR-149). A chat turn registered itself when it began; another run of the Lead does
    /// now, kept while the returned guard lives. Workers are never registered: they are not messaged (ADR-104).
    pub(super) fn steer_queue(
        &self,
        project: ProjectId,
        agent: AgentId,
        run: RunId,
    ) -> Result<(Option<agenium_backend::Steer>, Option<SteerGuard<'_>>)> {
        let registered = self
            .steering
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&agent)
            .and_then(|qs| qs.iter().find(|s| s.run == run))
            .map(|s| s.q.clone());
        if registered.is_some() {
            return Ok((registered, None));
        }
        if self.app.team_of_project(project)?.and_then(|(t, _)| t.lead) != Some(agent) {
            return Ok((None, None));
        }
        let (guard, q) = SteerGuard::register(self, project, agent, run);
        Ok((Some(q), Some(guard)))
    }

    /// Whether a run of `lead` is going on.
    pub(super) fn lead_busy(&self, lead: AgentId) -> bool {
        self.steering
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&lead)
            .is_some_and(|qs| !qs.is_empty())
    }

    /// Registers `run`, the chat turn about to start, when no run of `lead` is going on: one step under the steering
    /// lock, so two messages sent at once start one turn. `None` = the Lead is busy (the messages wait in the queue).
    pub(super) fn register_if_idle(
        &self,
        project: ProjectId,
        lead: AgentId,
        run: RunId,
    ) -> Option<SteerGuard<'_>> {
        let mut map = self.steering.lock().unwrap_or_else(|e| e.into_inner());
        let qs = map.entry(lead).or_default();
        if !qs.is_empty() {
            return None;
        }
        qs.push(SteerQueue {
            run,
            q: Default::default(),
        });
        Some(SteerGuard {
            rt: self,
            project,
            agent: lead,
            run,
        })
    }
}
