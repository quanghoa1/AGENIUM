//! Scheduling: one round of work/review/planner/inbox runs per project, pausing, resuming and driving a project to
//! quiescence. Split out of `runtime/mod.rs`.

use super::*;

/// How often a round with runs going looks again at agents set aside (their quota, an unreachable provider).
const RECHECK: std::time::Duration = std::time::Duration::from_secs(30);

impl Runtime {
    /// One scheduling round: start work runs for ready tickets (bounded parallelism), then review runs for Resolved
    /// tickets awaiting the Lead, then learn from the chat with the Lead if it has gone quiet. Returns the runs
    /// executed.
    pub async fn tick(&self, project: ProjectId) -> Result<Vec<RunSummary>> {
        self.agents_back();
        // ADR-166: what closed tickets added to the project map is confirmed, what cancelled ones added goes.
        // It reads files: off the async workers.
        let app = self.app.clone();
        match tokio::task::spawn_blocking(move || app.map_settle(project)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!(error = %e, "project map not settled"),
            Err(e) => tracing::warn!(error = %e, "project map not settled"),
        }
        // Before work runs: a ticket the user reopened is learned from while it is still Reopened (ADR-147).
        self.learn_owed(project).await;
        // The user's queued messages first: the user waits on them, the inbox and the work do not (ADR-149).
        let mut out: Vec<RunSummary> = self.answer_left(project).await.into_iter().collect();
        out.extend(self.tick_runs(project).await?);
        if let Err(e) = self.learn_from_chat(project, false).await {
            tracing::warn!(error = %e, "learning from the chat failed");
        }
        Ok(out)
    }

    /// Messages queued while the Lead was busy get its next chat turn once it is free (ADR-149), unless the project is
    /// paused (they wait for it to go on). Messages from before a restart are in the queue too (ADR-147). A turn that
    /// fails puts them back; after MAX_FAILED_RUNS such turns in a row they wait for the user's next message instead
    /// of a failing turn every round.
    async fn answer_left(&self, project: ProjectId) -> Option<RunSummary> {
        let pending = self
            .app
            .undelivered_lead_messages(project)
            .is_ok_and(|m| !m.is_empty());
        let failures = self
            .left_failures
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&project)
            .copied()
            .unwrap_or(0);
        if !pending || failures >= MAX_FAILED_RUNS || self.project_cancel(project).is_cancelled() {
            return None;
        }
        let lead = self.app.team_of_project(project).ok().flatten()?.0.lead?;
        if self.lead_busy(lead) {
            return None;
        }
        match self.deliver_queued(project).await {
            Ok(s) if s.kind == "queued" => None,
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!(error = %e, "answering the queued messages failed");
                *self
                    .left_failures
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(project)
                    .or_default() += 1;
                None
            }
        }
    }

    /// Agents set aside for their quota (ADR-135) or an unreachable provider (ADR-158) whose wait is over go on.
    fn agents_back(&self) {
        if let Err(e) = self.quota_back_round(std::time::SystemTime::now()) {
            tracing::warn!(error = %e, "resuming agents after their quota failed");
        }
        if let Err(e) = self.offline_back_round(std::time::SystemTime::now()) {
            tracing::warn!(error = %e, "trying unreachable agents again failed");
        }
    }

    async fn tick_runs(&self, project: ProjectId) -> Result<Vec<RunSummary>> {
        let mut out = Vec::new();
        // No run of this project is active between rounds: retry git operations that failed during one.
        let app = self.app.clone();
        tokio::task::spawn_blocking(move || app.replay_pending_ops(project))
            .await
            .map_err(|e| OrchError::Invalid(e.to_string()))??;
        // Team mode: the SLA of blocking questions, then the Lead's inbox (before work, so answers and new
        // assignments are picked up by this round's work runs).
        self.app.check_sla(project, chrono::Utc::now())?;
        // The user can stop the project while a phase runs: no later phase starts then (ADR-63).
        let paused = || self.project_cancel(project).is_cancelled();
        if paused() {
            return Ok(out);
        }
        if let Some(s) = self.planner_round(project).await? {
            out.push(s);
        }
        if paused() {
            return Ok(out);
        }
        if let Some(s) = self.inbox_round(project).await? {
            out.push(s);
        }
        if paused() {
            return Ok(out);
        }
        let max = self.app.config().runtime.max_parallel_agents.max(1) as usize;
        let lead = self.app.team_of_project(project)?.and_then(|(t, _)| t.lead);
        // Work runs and the Lead's reviews go on together (ADR-158): a review starts as soon as a report is in, not
        // once every work run of the round ended, and a worker its review freed starts its next ticket while the others
        // still run. One review at a time (one Lead); a worker waiting for its review takes nothing new (sched).
        enum Step {
            Work(TicketId, Result<RunSummary>),
            Review(Result<RunSummary>),
            Inbox(Result<Option<RunSummary>>),
        }
        let mut running: futures::stream::FuturesUnordered<
            std::pin::Pin<Box<dyn std::future::Future<Output = Step> + Send + '_>>,
        > = Default::default();
        // Tickets started this round: not started twice (a failed start or a reopen goes on in the next round, after
        // the op replay).
        let mut working: std::collections::HashSet<TicketId> = Default::default();
        // Agents with a run of this round going: never two at once (a Solo agent reviews only after its work run).
        let mut busy: std::collections::HashMap<TicketId, AgentId> = Default::default();
        let mut reviewed: std::collections::HashSet<TicketId> = Default::default();
        let mut reviewing = false;
        // The Lead's inbox is read while the round goes on too (a question or a permission asked now must not wait
        // for every long run), once per new state of the inbox.
        let mut inboxing = false;
        let mut inbox_seen = inbox_round::inbox_signature(&self.app.lead_inbox(project)?);
        // A store error stops scheduling, but the runs already going are drained, not dropped (dropping ends them).
        let mut failed: Option<OrchError> = None;
        loop {
            if failed.is_none() && !paused() {
                let scheduled = (|| -> Result<()> {
                    if busy.len() < max {
                        for r in self.app.ready_tickets(project)? {
                            if busy.len() >= max {
                                break;
                            }
                            if working.contains(&r.ticket.id)
                                || busy.values().any(|a| *a == r.agent)
                                || ((reviewing || inboxing) && lead == Some(r.agent))
                            {
                                continue;
                            }
                            if self.ready_to_autostart(r.ticket.id)?
                                || self.stall_nudge(&r.ticket)?
                            {
                                let id = r.ticket.id;
                                working.insert(id);
                                busy.insert(id, r.agent);
                                let key = r.ticket.key.clone();
                                running.push(Box::pin(async move {
                                    Step::Work(id, self.run_ticket(project, &key).await)
                                }));
                            }
                        }
                    }
                    if let (false, false, Some(lead)) = (reviewing, inboxing, lead) {
                        if !busy.values().any(|a| *a == lead)
                            && self.app.agent(lead)?.status == AgentStatus::Idle
                        {
                            if let Some(t) = self.next_review(project, &reviewed)? {
                                reviewing = true;
                                reviewed.insert(t.id);
                                let key = t.key;
                                running.push(Box::pin(async move {
                                    Step::Review(self.review(project, &key).await)
                                }));
                            } else {
                                let items = self.app.lead_inbox(project)?;
                                let sig = inbox_round::inbox_signature(&items);
                                if !items.is_empty() && sig != inbox_seen {
                                    inbox_seen = sig;
                                    inboxing = true;
                                    running.push(Box::pin(async move {
                                        Step::Inbox(self.inbox_round(project).await)
                                    }));
                                }
                            }
                        }
                    }
                    Ok(())
                })();
                if let Err(e) = scheduled {
                    tracing::warn!(error = %e, "scheduling stopped for this round; the runs going are drained");
                    failed = Some(e);
                }
            }
            // A long run must not hold back an agent whose wait is over (the research run: a Lead set aside for an
            // overloaded provider waited 18 minutes for a worker's run to end): every RECHECK, they are looked at.
            let step =
                match tokio::time::timeout(RECHECK, futures::StreamExt::next(&mut running)).await {
                    Ok(Some(step)) => step,
                    Ok(None) => break,
                    Err(_) => {
                        self.agents_back();
                        if let Err(e) = self.app.check_sla(project, chrono::Utc::now()) {
                            tracing::warn!(error = %e, "the SLA check failed");
                        }
                        continue;
                    }
                };
            match step {
                Step::Work(id, r) => {
                    busy.remove(&id);
                    match r {
                        Ok(s) => out.push(s),
                        Err(e) => tracing::warn!(error = %e, "work run failed to start"),
                    }
                }
                Step::Review(r) => {
                    reviewing = false;
                    match r {
                        Ok(s) => out.push(s),
                        Err(e) => tracing::warn!(error = %e, "review run failed"),
                    }
                }
                Step::Inbox(r) => {
                    inboxing = false;
                    match r {
                        Ok(s) => out.extend(s),
                        Err(e) => tracing::warn!(error = %e, "Lead inbox run failed"),
                    }
                }
            }
        }
        if let Some(e) = failed {
            return Err(e);
        }
        Ok(out)
    }

    /// The next Resolved ticket the Lead should review now (not yet decided, or approved and merged or without code
    /// and not closed), skipping those reviewed this round and those that wait for the user.
    fn next_review(
        &self,
        project: ProjectId,
        reviewed: &std::collections::HashSet<TicketId>,
    ) -> Result<Option<Ticket>> {
        for t in self.app.tickets(project)? {
            if reviewed.contains(&t.id) {
                continue;
            }
            let needs_review =
                t.status == TicketStatus::Resolved && (!t.approved || (t.merged || !t.has_code));
            if needs_review
                && !self.review_stalled(&t)?
                && (self.ready_to_autostart(t.id)? || self.stall_nudge(&t)?)
            {
                return Ok(Some(t));
            }
        }
        Ok(None)
    }

    /// Cancels `cancel` once the ticket is in a terminal state; the watch ends when the returned guard is dropped.
    pub(super) fn stop_when_ended(&self, ticket: TicketId, cancel: Cancel) -> AbortOnDrop {
        let app = self.app.clone();
        AbortOnDrop(tokio::spawn(async move {
            loop {
                tokio::time::sleep(TICKET_WATCH).await;
                match app.store().read(|c| tickets::get(c, ticket)) {
                    Ok(t) if t.status.is_terminal() => {
                        tracing::info!(ticket = %t.key, status = %t.status, "the ticket ended: its run stops");
                        cancel.cancel();
                        return;
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "the ticket of a run could not be read"),
                }
            }
        }))
    }

    /// The stop flag of a project's work.
    pub fn project_cancel(&self, project: ProjectId) -> Cancel {
        self.project_cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(project)
            .or_insert_with(|| self.cancel.child())
            .clone()
    }

    /// The user stops a project: its runs stop after the current step and no new one starts (ADR-63).
    pub fn pause_project(&self, project: ProjectId) -> Result<()> {
        self.app.set_project_status(
            project,
            agenium_store::core::projects::ProjectStatus::Paused,
        )?;
        self.project_cancel(project).cancel();
        Ok(())
    }

    /// The user lets a paused project go on.
    pub fn resume_project(&self, project: ProjectId) -> Result<()> {
        self.project_cancel(project).reset();
        self.app.set_project_status(
            project,
            agenium_store::core::projects::ProjectStatus::Active,
        )?;
        Ok(())
    }

    /// One pass of the continuous team loop for a project (ADR-63): a round when the project runs, then its status
    /// follows its work (Completed once everything is done, Active again when new work appears).
    pub async fn work(&self, project: ProjectId) -> Result<Vec<RunSummary>> {
        use agenium_store::core::projects::ProjectStatus;
        let state = self.app.run_state(project)?;
        // A project that is done (the user closed its last ticket) still learns from what made it so (ADR-147).
        if state != crate::views::RunState::Running {
            self.learn_owed(project).await;
        }
        let out = match state {
            crate::views::RunState::Running => self.tick(project).await?,
            crate::views::RunState::Paused => Vec::new(),
            // Closed by the user (ADR-130): nothing runs and its status stays as it was closed.
            crate::views::RunState::Closed => return Ok(Vec::new()),
            // Not running (yet, or any more): the Lead still answers what reached it after its last step (ADR-104).
            _ => self.answer_left(project).await.into_iter().collect(),
        };
        let status = self.app.project(project)?.status;
        let done = self.app.run_state(project)? == crate::views::RunState::Done;
        if done && status == ProjectStatus::Active {
            // ADR-147: done means handed over: the report is written first, then the project completes and the user
            // is told where the report is. A report that cannot be written leaves the project Active (tried again).
            match self.app.save_handover(project) {
                Ok(file) => {
                    self.app
                        .set_project_status_as(project, ProjectStatus::Completed, "system")?;
                    // The Lead says so in the chat, from the report (once: the project is Completed from now on).
                    self.summarize_done(project, &file).await;
                    let shown = file.display().to_string();
                    self.app.store().commit(None, "system", |w| {
                        w.emit(DomainEvent::NeedsUser {
                            ticket: None,
                            reason: format!(
                                "The project is done: its handover report (requirements, results, checks, reviews, \
                                 risks) is in {shown}. Read it, then close the project."
                            ),
                        })
                    })?;
                }
                Err(e) => tracing::warn!(error = %e, "the handover report could not be written"),
            }
        } else if !done && status == ProjectStatus::Completed {
            self.app
                .set_project_status_as(project, ProjectStatus::Active, "system")?;
        }
        Ok(out)
    }

    /// Repeats [`tick`](Self::tick) until nothing runs (or `max_rounds`).
    pub async fn drive(&self, project: ProjectId, max_rounds: usize) -> Result<Vec<RunSummary>> {
        let mut all = Vec::new();
        for _ in 0..max_rounds {
            let r = self.tick(project).await?;
            if r.is_empty() {
                break;
            }
            all.extend(r);
        }
        Ok(all)
    }
}
