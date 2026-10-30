//! The Lead's (or Solo agent's) own runs: messages to a running Lead, chat turns with the user, and inbox runs.
//! Split out of `runtime/mod.rs`.

use super::*;

impl Runtime {
    /// Delivers `text` to the project's Lead (or Solo agent) if it is running now: it reads it before its next model
    /// request (ADR-104). `false` when it is not running (or runs through a vendor CLI), so a new turn is needed.
    pub fn steer(&self, project: ProjectId, text: &str) -> Result<bool> {
        let Some(lead) = self.app.team_of_project(project)?.and_then(|(t, _)| t.lead) else {
            return Ok(false);
        };
        let map = self.steering.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = map.get(&lead).and_then(|qs| recipient(qs)) else {
            return Ok(false);
        };
        s.q.lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(text.to_owned());
        tracing::info!(%project, "message delivered to the running Lead");
        Ok(true)
    }

    /// One chat turn of the user with the Lead/Solo (discovery, spec, plan, ticket creation). The conversation is kept
    /// per project in the Lead's private database. When the Lead is running already, the message goes to that run.
    pub async fn chat(&self, project: ProjectId, user_text: &str) -> Result<RunSummary> {
        // ADR-147: the message is stored before anything that waits, so a crash before the Lead's turn loses nothing.
        if !user_text.trim().is_empty() && self.app.team_of_project(project)?.is_some() {
            self.app.post_lead_message(project, user_text)?;
        }
        self.deliver_to_lead(project, user_text).await
    }

    /// The Lead's turn on a message already stored with [`App::post_lead_message`] (`chat` stores and delivers).
    pub async fn deliver_to_lead(&self, project: ProjectId, user_text: &str) -> Result<RunSummary> {
        // Messages left after the Lead's last step open this turn; a turn that fails keeps them for the next one.
        let left = self
            .steer_left
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&project)
            .unwrap_or_default();
        let text = if left.is_empty() {
            user_text.to_owned()
        } else {
            format!("{}\n\n{user_text}", left.join("\n\n"))
        };
        let mut opened = OpenRun::new(&self.app);
        let r = self.chat_run(project, &text, &mut opened.run).await;
        if r.is_ok() {
            self.left_failures
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&project);
        } else if !left.is_empty() {
            let mut m = self.steer_left.lock().unwrap_or_else(|e| e.into_inner());
            let v = m.entry(project).or_default();
            v.splice(0..0, left);
        }
        self.end_on_error(opened.take(), r)
    }

    async fn chat_run(
        &self,
        project: ProjectId,
        user_text: &str,
        opened: &mut Option<RunId>,
    ) -> Result<RunSummary> {
        let (team, _) = self
            .app
            .team_of_project(project)?
            .ok_or_else(|| OrchError::Invalid("the project has no team".into()))?;
        let lead = self.app.agent(
            team.lead
                .ok_or_else(|| OrchError::Invalid("the team has no Lead".into()))?,
        )?;
        let run = RunId::new();
        // Before anything that waits: a message sent while this turn starts reaches it, not a second turn (ADR-104).
        let Some(_steering) = self.steer_or_register(project, lead.id, run, user_text) else {
            let name = lead.name;
            return Ok(RunSummary {
                run: RunId::new(),
                kind: "steer".into(),
                ticket: None,
                agent: name.clone(),
                outcome: "Delivered".into(),
                steps: 0,
                usage: Default::default(),
                text: format!("{name} is working; your message reaches it before its next step."),
            });
        };
        let p = self.app.project(project)?;
        let root = PathBuf::from(&p.path);
        let mem = self.agent_memory(lead.id)?;
        let session = learn::chat_session(project);
        // Older turns of a long chat are seen as a summary (ADR-106); the stored conversation stays whole.
        let (history, summary_usage) = self.chat_history(&lead, &mem, project).await?;
        let run = self
            .app
            .store()
            .commit(None, "system", |w| -> Result<RunId> {
                let id = run;
                runs::insert(
                    w.tx,
                    &runs::Run {
                        id,
                        ticket: None,
                        agent: lead.id,
                        provider: lead.model.provider.clone(),
                        model: lead.model.model.clone(),
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
                    agent: lead.id,
                    ticket: None,
                })?;
                Ok(id)
            })?
            .value;
        *opened = Some(run);
        let state = self.project_state_text(project)?;
        let soul = self.soul_and_role(&lead);
        let solo = team.solo;
        let started = Instant::now();
        // The turn's own context (worktree, the team's state now) opens the user's message, which is stored exactly as
        // sent: the next turn replays the same bytes, so the reply's thinking stays valid and the history stays a
        // cached prefix (ADR-113).
        let roster = self
            .app
            .apply(
                &crate::engine::CallCtx {
                    project,
                    actor: Actor::User,
                    intent_id: None,
                    run: None,
                },
                Intent::GetTeam(Default::default()),
            )
            .map(|o| o.result)
            .ok();
        let mut run_context = format!("Run context\n- Worktree: {}", root.display());
        if let Some(t) = &roster {
            run_context.push_str(&format!("\n- Team: {t}"));
        }
        let msg = Message::user_text(format!(
            "{run_context}\n\n{user_text}\n\n<project_state>\n{state}\n</project_state>"
        ));
        let builder = |ctx: &ToolCtx, specs: &[ToolSpec]| -> Result<WorkOrder> {
            let (manifest, _) = self.manifest(&lead, solo, ctx, specs, None, &ctx.fetch_allow);
            let system = vec![
                SystemBlock { text: prompt::system_rules(true), cache: false },
                SystemBlock { text: format!("{soul}\n\n# Talking with the user\nThe user writes to you directly. Clarify the request, then propose_spec, create_plan, create_ticket and assign_ticket as needed (a Solo agent assigns tickets to itself). You do not edit files in this conversation: once a ticket is assigned, the system starts a separate work run with file and shell tools in the ticket's worktree. Reply to the user in their language; everything you write into tickets and plans is English."), cache: true },
                SystemBlock { text: manifest, cache: false },
            ];
            let mut messages = history.clone();
            messages.push(msg.clone());
            Ok(WorkOrder { system, messages })
        };
        let read_only: Vec<Arc<dyn Tool>> = agenium_tools::work_tools()
            .into_iter()
            .filter(|t| t.read_only() && t.name() != "fetch")
            .collect();
        // Chat history lives in the chat session, not in the run's session.
        let res = self
            .execute(
                RunKind::Chat,
                project,
                &lead,
                solo,
                run,
                None,
                &root,
                builder,
                read_only,
                &[
                    ProtocolTool::ProposeSkillUpdate,
                    ProtocolTool::ProposeNewSkill,
                    ProtocolTool::SubmitReport,
                    ProtocolTool::PostProgress,
                    ProtocolTool::AskQuestion,
                    ProtocolTool::RequestPermission,
                    ProtocolTool::SuggestLink,
                ],
            )
            .await;
        let (report, cost) = res?;
        for m in std::iter::once(&msg).chain(report.messages.iter()) {
            if let Ok(v) = serde_json::to_value(m) {
                mem.append_message(&session, &v)?;
            }
        }
        // When the chat goes quiet, the scheduler learns from it (ADR-98).
        mem.set_meta(&format!("chat-last:{project}"), &now_iso())?;
        let text = report
            .messages
            .iter()
            .rev()
            .find(|m| m.role == agenium_llm::Role::Assistant && !m.text().is_empty())
            .map(|m| m.text())
            .unwrap_or_default();
        let mut usage = bridge::summary(&report.usage, cost, started.elapsed().as_millis() as u64);
        // The summary of the older chat this turn needed (ADR-106) is part of its cost.
        usage.add(&summary_usage);
        self.app
            .finish_run(run, &report.outcome.label(), report.steps, &usage)?;
        self.agent_state_after(&lead, None, &report.outcome)?;
        let text = match (&report.outcome, text.is_empty()) {
            (Outcome::QuotaExhausted { message }, _) => {
                format!("The provider reports exhausted quota or credit: {message}")
            }
            (Outcome::Offline { message }, _) => format!("The model cannot be used: {message}"),
            (Outcome::Failed { message }, true) => format!("The model call failed: {message}"),
            _ => text,
        };
        Ok(RunSummary {
            run,
            kind: "chat".into(),
            ticket: None,
            agent: lead.name.clone(),
            outcome: report.outcome.label(),
            steps: report.steps,
            usage,
            text,
        })
    }

    /// Lead run on its inbox (team mode): answer or escalate worker questions, decide permission requests inside the
    /// allowlist, turn approved plan items into tickets and assign them. `None` when the inbox is empty.
    pub async fn inbox(&self, project: ProjectId) -> Result<Option<RunSummary>> {
        let mut opened = OpenRun::new(&self.app);
        let r = self.inbox_run(project, &mut opened.run).await;
        self.end_on_error(opened.take(), r)
    }

    async fn inbox_run(
        &self,
        project: ProjectId,
        opened: &mut Option<RunId>,
    ) -> Result<Option<RunSummary>> {
        let items = self.app.lead_inbox(project)?;
        if items.is_empty() {
            return Ok(None);
        }
        let (team, _) = self
            .app
            .team_of_project(project)?
            .ok_or_else(|| OrchError::Invalid("the project has no team".into()))?;
        let lead = self.app.agent(
            team.lead
                .ok_or_else(|| OrchError::Invalid("the team has no Lead".into()))?,
        )?;
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
                        agent: lead.id,
                        provider: lead.model.provider.clone(),
                        model: lead.model.model.clone(),
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
                    agent: lead.id,
                    ticket: None,
                })?;
                Ok(id)
            })?
            .value;
        *opened = Some(run);
        // The tickets the items are about, with their comments (questions need the ticket's context).
        let mut seen = std::collections::HashSet::new();
        let mut tickets_text = String::new();
        for k in items.iter().filter_map(|i| i.ticket.as_ref()) {
            if seen.insert(k.clone()) {
                let t = self.app.ticket(project, k)?;
                let cs = self.app.store().read(|c| comments::for_ticket(c, t.id))?;
                tickets_text.push_str(&format!(
                    "{}\n{}\n",
                    prompt::ticket_text(&t, None),
                    prompt::comments_text(&cs)
                ));
            }
        }
        let task = format!(
            "Lead inbox, {} item(s), most urgent first:\n{}\n\nHandle every item with your tools: answer_question \
             (or escalate = true when only the user can decide: scope, acceptance criteria, facts you do not have), \
             approve_permission, create_ticket for plan items (one ticket per item unless it is too large; keep \
             depends_on in line with the plan) and assign_ticket to a team member whose role and tags match (see \
             get_team). The run ends by itself once the inbox is empty.\n\n<project_state>\n{}\n</project_state>\n\n{}",
            items.len(),
            items
                .iter()
                .enumerate()
                .map(|(i, it)| format!("{}. {}", i + 1, it.text))
                .collect::<Vec<_>>()
                .join("\n"),
            self.project_state_text(project)?,
            tickets_text
        );
        let soul = self.soul_and_role(&lead);
        let started = Instant::now();
        let builder = |ctx: &ToolCtx, specs: &[ToolSpec]| -> Result<WorkOrder> {
            let roster = self
                .app
                .apply(
                    &crate::engine::CallCtx {
                        project,
                        actor: Actor::User,
                        intent_id: None,
                        run: None,
                    },
                    Intent::GetTeam(Default::default()),
                )
                .map(|o| o.result)
                .ok();
            let (manifest, run_context) =
                self.manifest(&lead, false, ctx, specs, roster, &ctx.fetch_allow);
            Ok(WorkOrder {
                system: vec![
                    SystemBlock {
                        text: prompt::system_rules(true),
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
        let res = self
            .execute(
                RunKind::Inbox,
                project,
                &lead,
                false,
                run,
                None,
                &root,
                builder,
                read_only,
                &[
                    ProtocolTool::ProposeSkillUpdate,
                    ProtocolTool::ProposeNewSkill,
                    ProtocolTool::ProposeSpec,
                    ProtocolTool::ReviewTicket,
                    ProtocolTool::CloseTicket,
                ],
            )
            .await;
        let (report, cost) = res?;
        let usage = bridge::summary(&report.usage, cost, started.elapsed().as_millis() as u64);
        let label = report.outcome.label();
        self.app.finish_run(run, &label, report.steps, &usage)?;
        self.agent_state_after(&lead, None, &report.outcome)?;
        self.app
            .run_log(run, "run", serde_json::json!({ "kind": "inbox" }))?;
        let text = report
            .messages
            .iter()
            .rev()
            .find(|m| m.role == agenium_llm::Role::Assistant && !m.text().is_empty())
            .map(|m| m.text())
            .unwrap_or_default();
        Ok(Some(RunSummary {
            run,
            kind: "inbox".into(),
            ticket: None,
            agent: lead.name.clone(),
            outcome: label,
            steps: report.steps,
            usage,
            text,
        }))
    }
}
