//! Messages to a Lead while it runs: each run's queue, and where a message goes when the run ends.

use super::*;

/// The message queue of one run of a Lead.
pub(super) struct SteerQueue {
    run: RunId,
    chat: bool,
    pub(super) q: agenium_backend::Steer,
}

/// The queue a message to a Lead goes to: its chat turn's (the user's conversation), else its latest run's.
pub(super) fn recipient(qs: &[SteerQueue]) -> Option<&SteerQueue> {
    qs.iter().rev().find(|s| s.chat).or(qs.last())
}

/// Keeps a Lead's run's message queue registered while the run lasts. Messages that arrive after its last step go to
/// another run of the Lead that goes on, else to the project's next chat turn.
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
        chat: bool,
    ) -> (Self, agenium_backend::Steer) {
        let q = agenium_backend::Steer::default();
        rt.steering
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(agent)
            .or_default()
            .push(SteerQueue {
                run,
                chat,
                q: q.clone(),
            });
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
        // Only this run's own queue is removed (another run of the Lead keeps its own), and removing it and handing on
        // what is left happen under the same lock as `steer`, so no message is lost or reaches a queue nobody reads.
        let mut map = self.rt.steering.lock().unwrap_or_else(|e| e.into_inner());
        let Some(qs) = map.get_mut(&self.agent) else {
            return;
        };
        let Some(i) = qs.iter().position(|s| s.run == self.run) else {
            return;
        };
        let mine = qs.remove(i);
        let left = std::mem::take(&mut *mine.q.lock().unwrap_or_else(|e| e.into_inner()));
        if !left.is_empty() {
            match recipient(qs) {
                Some(s) => s.q.lock().unwrap_or_else(|e| e.into_inner()).extend(left),
                None => self
                    .rt
                    .steer_left
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(self.project)
                    .or_default()
                    .extend(left),
            }
        }
        if qs.is_empty() {
            map.remove(&self.agent);
        }
    }
}

impl Runtime {
    /// The message queue of `run`: only the project's Lead (a Solo agent is its own team's Lead) can be messaged while
    /// it runs (ADR-104). A chat turn registered its queue when it began; another run of the Lead registers one now,
    /// kept while the returned guard lives.
    pub(super) fn steer_queue(
        &self,
        project: ProjectId,
        agent: AgentId,
        run: RunId,
        kind: RunKind,
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
        let (guard, q) = SteerGuard::register(self, project, agent, run, kind == RunKind::Chat);
        Ok((Some(q), Some(guard)))
    }

    /// Delivers `text` to a running run of the Lead, or else registers the queue of `run`, the chat turn about to
    /// start: one step under the steering lock, so a message sent while the turn starts reaches it instead of starting
    /// a second turn. `None` = delivered.
    pub(super) fn steer_or_register(
        &self,
        project: ProjectId,
        lead: AgentId,
        run: RunId,
        text: &str,
    ) -> Option<SteerGuard<'_>> {
        let mut map = self.steering.lock().unwrap_or_else(|e| e.into_inner());
        let qs = map.entry(lead).or_default();
        if let Some(s) = recipient(qs) {
            s.q.lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(text.to_owned());
            tracing::info!(%project, "message delivered to the running Lead");
            return None;
        }
        qs.push(SteerQueue {
            run,
            chat: true,
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
