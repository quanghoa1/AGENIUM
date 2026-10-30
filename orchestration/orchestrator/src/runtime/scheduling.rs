//! Scheduling: one round of work/review/planner/inbox runs per project, pausing, resuming and driving a project to
//! quiescence. Split out of `runtime/mod.rs`.

use super::*;

impl Runtime {
    /// One scheduling round: start work runs for ready tickets (bounded parallelism), then review runs for Resolved
    /// tickets awaiting the Lead, then learn from the chat with the Lead if it has gone quiet. Returns the runs
    /// executed.
    pub async fn tick(&self, project: ProjectId) -> Result<Vec<RunSummary>> {
        // Agents whose quota should be back go on (ADR-135).
        if let Err(e) = self.quota_back_round(std::time::SystemTime::now()) {
            tracing::warn!(error = %e, "resuming agents after their quota failed");
        }
        // Before work runs: a ticket the user reopened is learned from while it is still Reopened (ADR-147).
        self.learn_owed(project).await;
        let mut out = self.tick_runs(project).await?;
        out.extend(self.answer_left(project).await);
        if let Err(e) = self.learn_from_chat(project, false).await {
            tracing::warn!(error = %e, "learning from the chat failed");
        }
        Ok(out)
    }

    /// Messages that reached the Lead after its last step get their own turn (ADR-104), unless the project is paused
    /// (the next turn starts with them then). A turn that fails keeps them; after MAX_FAILED_RUNS such turns in a row
    /// they wait for the user's next message instead of a failing turn every round.
    async fn answer_left(&self, project: ProjectId) -> Option<RunSummary> {
        self.replay_lead_messages(project);
        let pending = self
            .steer_left
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&project)
            .is_some_and(|v| !v.is_empty());
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
        match self.chat(project, "").await {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!(error = %e, "answering messages sent during a run failed");
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

    /// Once per project and process: messages to the Lead stored before this runtime started and never delivered (the
    /// app stopped before a run of the Lead had them) go to its next turn (ADR-147).
    fn replay_lead_messages(&self, project: ProjectId) {
        if !self
            .lead_replayed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(project)
        {
            return;
        }
        let old: Vec<String> = match self.app.undelivered_lead_messages(project) {
            Ok(ms) => ms
                .into_iter()
                .filter(|m| m.created_at < self.started_at)
                .map(|m| m.text)
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "undelivered messages to the Lead not read");
                return;
            }
        };
        if !old.is_empty() {
            let mut m = self.steer_left.lock().unwrap_or_else(|e| e.into_inner());
            m.entry(project).or_default().splice(0..0, old);
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
        let mut ready = Vec::new();
        for r in self.app.ready_tickets(project)? {
            if ready.len() == max {
                break;
            }
            if self.ready_to_autostart(r.ticket.id)? || self.stall_nudge(&r.ticket)? {
                ready.push(r);
            }
        }
        let futs = ready
            .iter()
            .map(|r| self.run_ticket(project, &r.ticket.key));
        for r in futures_join_all(futs).await {
            match r {
                Ok(s) => out.push(s),
                Err(e) => tracing::warn!(error = %e, "work run failed to start"),
            }
        }
        // Lead reviews: Resolved and not yet decided (or approved+merged and not yet closed).
        let (team, _) = match self.app.team_of_project(project)? {
            Some(t) => t,
            None => return Ok(out),
        };
        let Some(lead) = team.lead else {
            return Ok(out);
        };
        for t in self.app.tickets(project)? {
            // Checked before each review: one that left the Lead out of quota or offline (or a pause) would make every
            // further review of this round fail the same way.
            if paused() || self.app.agent(lead)?.status != AgentStatus::Idle {
                break;
            }
            let needs_review =
                t.status == TicketStatus::Resolved && (!t.approved || (t.merged || !t.has_code));
            let gated = t.status == TicketStatus::Resolved && t.approved && t.has_code && !t.merged;
            if needs_review
                && !gated
                && !self.review_stalled(&t)?
                && (self.ready_to_autostart(t.id)? || self.stall_nudge(&t)?)
            {
                match self.review(project, &t.key).await {
                    Ok(s) => out.push(s),
                    Err(e) => tracing::warn!(error = %e, "review run failed"),
                }
            }
        }
        Ok(out)
    }

    /// Runs the team's planner when the latest spec is approved, there is no plan and no draft for that spec yet
    /// (ADR-59). Two failed runs for the same spec stop it and go to the user.
    async fn planner_round(&self, project: ProjectId) -> Result<Option<RunSummary>> {
        let Some(planner) = self.app.planner(project)? else {
            return Ok(None);
        };
        use agenium_store::core::plans;
        let (spec, any_plan) = self.app.store().read(|c| {
            Ok((
                plans::latest_spec(c, project)?,
                plans::latest_plan(c, project)?.is_some(),
            ))
        })?;
        let Some(spec) = spec.filter(|s| s.locked_at.is_some()) else {
            return Ok(None);
        };
        if any_plan
            || planner.status != AgentStatus::Idle
            || self.app.plan_draft(project, spec.version)?.is_some()
        {
            return Ok(None);
        }
        let key = (project, spec.version);
        let failures = *self
            .planner_failures
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
            .unwrap_or(&0);
        if failures >= MAX_FAILED_RUNS {
            return Ok(None);
        }
        // A run that fails before its loop counts like one without a draft (else it is retried every round); one the
        // user stopped does not count.
        let (s, outcome) = match self.plan_draft_run(project, &planner, &spec).await {
            Ok(s) => {
                let o = s.outcome.clone();
                (Some(s), o)
            }
            Err(e) => {
                tracing::warn!(error = %e, "planner run failed");
                (None, format!("Error:{e}"))
            }
        };
        if outcome != "Cancelled" && self.app.plan_draft(project, spec.version)?.is_none() {
            let n = failures + 1;
            self.planner_failures
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(key, n);
            if n >= MAX_FAILED_RUNS {
                self.app.store().commit(None, "system", |w| {
                    w.emit(DomainEvent::NeedsUser {
                        ticket: None,
                        reason: format!(
                            "the planner {} could not draft plan v1 for spec v{} ({}); the Lead can create the plan in chat",
                            planner.name, spec.version, outcome
                        ),
                    })
                    .map_err(OrchError::from)
                })?;
            }
        }
        Ok(s)
    }

    /// The planner's run: read the locked spec and the repository, submit a plan draft with its rationale.
    async fn plan_draft_run(
        &self,
        project: ProjectId,
        planner: &agenium_store::core::agents::Agent,
        spec: &agenium_store::core::plans::SpecRow,
    ) -> Result<RunSummary> {
        let mut opened = OpenRun::new(&self.app);
        let r = self
            .plan_draft(project, planner, spec, &mut opened.run)
            .await;
        self.end_on_error(opened.take(), r)
    }

    async fn plan_draft(
        &self,
        project: ProjectId,
        planner: &agenium_store::core::agents::Agent,
        spec: &agenium_store::core::plans::SpecRow,
        opened: &mut Option<RunId>,
    ) -> Result<RunSummary> {
        let p = self.app.project(project)?;
        let root = PathBuf::from(&p.path);
        let run = self
            .app
            .store()
            .commit(None, "system", |w| -> Result<RunId> {
                let id = RunId::new();
                runs::insert(
                    w.tx,
                    &runs::Run {
                        id,
                        ticket: None,
                        agent: planner.id,
                        provider: planner.model.provider.clone(),
                        model: planner.model.model.clone(),
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
                    agent: planner.id,
                    ticket: None,
                })?;
                Ok(id)
            })?
            .value;
        *opened = Some(run);
        // The roles the plan can use: the team's workers except the planner.
        let roles: Vec<String> = self
            .app
            .team_of_project(project)?
            .map(|(_, m)| m)
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m.role_kind == RoleKind::Worker && m.id != planner.id)
            .map(|m| format!("- {} (role {}, tags {})", m.name, m.role, m.tags.join(", ")))
            .collect();
        let task = format!(
            "Draft plan v1 for the approved requirement spec v{v}.\n\n## Spec v{v}\n{spec}\n\n## Team members who will do the work\n{roles}\n\n\
             Read the repository with your read-only tools first. Then call submit_plan_draft once: milestones; items with \
             acceptance criteria (ids AC-n), trace_to (requirement ids of this spec), role (one of the roles above), \
             depends_on (item ids), estimate and scope_paths (items touching the same paths must depend on each other); \
             and the rationale of the split. You do not create tickets; the Lead reviews your draft.",
            v = spec.version,
            spec = serde_json::to_string_pretty(&spec.body).unwrap_or_default(),
            roles = roles.join("\n"),
        );
        let soul = self.soul_and_role(planner);
        let started = Instant::now();
        let builder = |ctx: &ToolCtx, specs: &[ToolSpec]| -> Result<WorkOrder> {
            let (manifest, run_context) =
                self.manifest(planner, false, ctx, specs, None, &ctx.fetch_allow);
            Ok(WorkOrder {
                system: vec![
                    SystemBlock {
                        text: prompt::system_rules(false),
                        cache: false,
                    },
                    SystemBlock {
                        text: soul.clone(),
                        cache: true,
                    },
                    SystemBlock {
                        text: manifest,
                        cache: false,
                    },
                ],
                messages: vec![Message::user_text(format!("{run_context}\n\n{task}"))],
            })
        };
        let read_only: Vec<Arc<dyn Tool>> = agenium_tools::work_tools()
            .into_iter()
            .filter(|t| t.read_only() && t.name() != "fetch")
            .collect();
        let others: Vec<ProtocolTool> = ProtocolTool::ALL
            .iter()
            .copied()
            .filter(|t| !matches!(t, ProtocolTool::SubmitPlanDraft | ProtocolTool::LoadSkill))
            .collect();
        let res = self
            .execute(
                RunKind::Plan,
                project,
                planner,
                false,
                run,
                None,
                &root,
                builder,
                read_only,
                &others,
            )
            .await;
        let (report, cost) = res?;
        let usage = bridge::summary(&report.usage, cost, started.elapsed().as_millis() as u64);
        let label = report.outcome.label();
        self.app.finish_run(run, &label, report.steps, &usage)?;
        self.agent_state_after(planner, None, &report.outcome)?;
        self.app
            .run_log(run, "run", serde_json::json!({ "kind": "plan" }))?;
        Ok(RunSummary {
            run,
            kind: "plan".into(),
            ticket: None,
            agent: planner.name.clone(),
            outcome: label,
            steps: report.steps,
            usage,
            text: String::new(),
        })
    }

    /// Runs the Lead on its inbox when the Lead is Idle and the same inbox did not fail twice in a row.
    async fn inbox_round(&self, project: ProjectId) -> Result<Option<RunSummary>> {
        let items = self.app.lead_inbox(project)?;
        if items.is_empty() {
            return Ok(None);
        }
        let Some((team, _)) = self.app.team_of_project(project)? else {
            return Ok(None);
        };
        let Some(lead) = team.lead else {
            return Ok(None);
        };
        if self.app.agent(lead)?.status != AgentStatus::Idle {
            return Ok(None);
        }
        let signature = items
            .iter()
            .map(|i| format!("{:?}:{:?}:{:?}", i.kind, i.ticket, i.comment))
            .collect::<Vec<_>>()
            .join("|");
        let failures = |m: &Mutex<std::collections::HashMap<ProjectId, (String, usize)>>| {
            m.lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&project)
                .filter(|(s, _)| *s == signature)
                .map_or(0, |(_, n)| *n)
        };
        if failures(&self.inbox_failures) >= MAX_FAILED_RUNS {
            return Ok(None);
        }
        // A run that fails before its loop counts like one that did not empty the inbox (else it is retried every
        // round); one the user stopped does not count.
        let (s, outcome) = match self.inbox(project).await {
            Ok(Some(s)) => {
                let o = s.outcome.clone();
                (Some(s), o)
            }
            Ok(None) => return Ok(None),
            Err(e) => {
                tracing::warn!(error = %e, "Lead inbox run failed");
                (None, format!("Error:{e}"))
            }
        };
        let mut m = self
            .inbox_failures
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if outcome == "Completed" {
            m.remove(&project);
        } else if outcome != "Cancelled" {
            let n = m
                .get(&project)
                .filter(|(sig, _)| *sig == signature)
                .map_or(0, |(_, n)| *n)
                + 1;
            m.insert(project, (signature, n));
            if n >= MAX_FAILED_RUNS {
                drop(m);
                self.app.store().commit(None, "system", |w| {
                    w.emit(DomainEvent::NeedsUser {
                        ticket: None,
                        reason: format!(
                            "the Lead could not handle its inbox ({} item(s)) in {n} runs ({}); look at the run log",
                            items.len(),
                            outcome
                        ),
                    })
                    .map_err(OrchError::from)
                })?;
            }
        }
        Ok(s)
    }

    /// Repeats [`tick`](Self::tick) until nothing runs (or `max_rounds`).
    /// The stop flag of a project's work.
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
                        .set_project_status(project, ProjectStatus::Completed)?;
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
                .set_project_status(project, ProjectStatus::Active)?;
        }
        Ok(out)
    }

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
