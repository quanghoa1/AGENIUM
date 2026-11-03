//! The Lead's (or Solo agent's) own runs: messages to a running Lead, chat turns with the user, and inbox runs.
//! Split out of `runtime/mod.rs`.

use super::*;

impl Runtime {
    /// When a project is done, its Lead (or Solo agent) tells the user in the chat what was delivered: the handover
    /// report summed up by the Lead's own model (API or CLI), in the language of the user's last message. A failure
    /// is only logged: the report and the "done" note are there either way.
    pub(super) async fn summarize_done(&self, project: ProjectId, report: &std::path::Path) {
        if let Err(e) = self.try_summarize_done(project, report).await {
            tracing::warn!(error = %e, "the summary of the done project was not written");
        }
    }

    async fn try_summarize_done(&self, project: ProjectId, report: &std::path::Path) -> Result<()> {
        let (team, _) = self
            .app
            .team_of_project(project)?
            .ok_or_else(|| OrchError::Invalid("the project has no team".into()))?;
        let lead = self.app.agent(
            team.lead
                .ok_or_else(|| OrchError::Invalid("the team has no Lead".into()))?,
        )?;
        let md: String = std::fs::read_to_string(report)
            .map_err(|e| OrchError::Invalid(format!("{}: {e}", report.display())))?
            .chars()
            .take(40_000)
            .collect();
        let mem = self.agent_memory(lead.id)?;
        let session = learn::chat_session(project);
        let last_user = mem
            .load_messages(&session)?
            .into_iter()
            .rev()
            .filter_map(|v| serde_json::from_value::<Message>(v).ok())
            .find(|m| m.role == agenium_llm::Role::User)
            .map(|m| m.text())
            .unwrap_or_default();
        let ask = format!(
            "The project is done. Its handover report:\n\n<report>\n{md}\n</report>\n\nWrite the user a short \
             summary of it (under 200 words, Markdown allowed): what was delivered, how it was checked, anything left \
             open or risky, and that the full report is in {}. Answer in the language of the user's last message:\n\
             <last_message>\n{last_user}\n</last_message>",
            report.display()
        );
        let a = self
            .side_chat(
                "done-summary",
                None,
                lead.backend,
                &lead.model,
                "You are the project's Lead, telling the user the work is done. Plain and short; propose no new work.",
                vec![Message::user_text(ask)],
                2_000,
            )
            .await?;
        if a.text.trim().is_empty() {
            return Ok(());
        }
        let v = serde_json::to_value(Message::assistant_text(a.text.trim()))
            .map_err(|e| OrchError::Invalid(e.to_string()))?;
        mem.append_message(&session, &v)?;
        Ok(())
    }

    /// Queues `text` for the project's Lead (or Solo agent) when it is running now (ADR-149): its next chat turn takes
    /// it, and until then the user can change or take it back. `false` when the Lead is not running (a chat turn
    /// answers at once instead); the text is not stored then.
    pub fn steer(&self, project: ProjectId, text: &str) -> Result<bool> {
        let Some(lead) = self.app.team_of_project(project)?.and_then(|(t, _)| t.lead) else {
            return Ok(false);
        };
        if !self.lead_busy(lead) {
            return Ok(false);
        }
        self.app.post_lead_message(project, text)?;
        tracing::info!(%project, "message queued for the busy Lead");
        Ok(true)
    }

    /// One chat turn of the user with the Lead/Solo (discovery, spec, plan, ticket creation). The conversation is kept
    /// per project in the Lead's private database. The message is stored first (ADR-147); when the Lead is busy it
    /// waits in the queue for the Lead's next chat turn (ADR-149).
    pub async fn chat(&self, project: ProjectId, user_text: &str) -> Result<RunSummary> {
        if !user_text.trim().is_empty() && self.app.team_of_project(project)?.is_some() {
            self.app.post_lead_message(project, user_text)?;
        }
        self.deliver_queued(project).await
    }

    /// The Lead's chat turn on every message in the queue, oldest first; when the Lead is busy they stay queued and
    /// the summary says so (`queued`). Messages a turn took and did not answer go back to the queue.
    pub async fn deliver_queued(&self, project: ProjectId) -> Result<RunSummary> {
        let (team, _) = self
            .app
            .team_of_project(project)?
            .ok_or_else(|| OrchError::Invalid("the project has no team".into()))?;
        let lead = self.app.agent(
            team.lead
                .ok_or_else(|| OrchError::Invalid("the team has no Lead".into()))?,
        )?;
        let run = RunId::new();
        let queued = |text: String| RunSummary {
            run: RunId::new(),
            kind: "queued".into(),
            ticket: None,
            agent: lead.name.clone(),
            outcome: "Queued".into(),
            steps: 0,
            usage: Default::default(),
            text,
        };
        // Under the steering lock: a message sent while this turn starts waits for the next one.
        let Some(_turn) = self.register_if_idle(project, lead.id, run) else {
            return Ok(queued(format!(
                "{} is working: your message waits in the queue (you can change it until then) and opens its next turn.",
                lead.name
            )));
        };
        let taken = self.app.take_lead_messages(project)?;
        if taken.messages.is_empty() {
            return Ok(queued(String::new()));
        }
        let mut opened = OpenRun::new(&self.app);
        let r = self
            .chat_run(project, &lead, run, &taken.text(), &mut opened.run)
            .await;
        if r.is_ok() {
            taken.keep();
            self.left_failures
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&project);
        }
        self.end_on_error(opened.take(), r)
    }

    async fn chat_run(
        &self,
        project: ProjectId,
        lead: &agenium_store::core::agents::Agent,
        run: RunId,
        user_text: &str,
        opened: &mut Option<RunId>,
    ) -> Result<RunSummary> {
        let (team, _) = self
            .app
            .team_of_project(project)?
            .ok_or_else(|| OrchError::Invalid("the project has no team".into()))?;
        let p = self.app.project(project)?;
        let root = PathBuf::from(&p.path);
        let mem = self.agent_memory(lead.id)?;
        let session = learn::chat_session(project);
        // Older turns of a long chat are seen as a summary (ADR-106); the stored conversation stays whole.
        let (history, summary_usage) = self.chat_history(lead, &mem, project).await?;
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
        // What the Lead remembers about what the user asks (ADR-155): the preferences and facts it learned from the
        // chat (ADR-98) among them. It goes in the turn's message, stored and replayed as sent like the project state:
        // a varying system block would make the whole chat history a cache miss at every turn. A memory the history
        // already carries is not repeated.
        let memory = self
            .recall_text(lead.id, project, run, None, user_text.to_owned())
            .await?;
        let seen: String = history.iter().map(|m| m.text()).collect();
        let fresh: Vec<&str> = memory
            .lines()
            .filter(|l| l.starts_with("- ") && !seen.contains(*l))
            .collect();
        let memory = if fresh.is_empty() {
            String::new()
        } else {
            format!(
                "{}\n{}\n\n",
                memory.lines().next().unwrap_or_default(),
                fresh.join("\n")
            )
        };
        let soul = self.soul_and_role(lead);
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
            "{run_context}\n\n{memory}{user_text}\n\n<project_state>\n{state}\n</project_state>"
        ));
        let builder = |ctx: &ToolCtx, specs: &[ToolSpec]| -> Result<WorkOrder> {
            let (manifest, _) = self.manifest(lead, solo, ctx, specs, None, &ctx.fetch_allow);
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
                lead,
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
        self.agent_state_after(lead, None, &report.outcome)?;
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
        // What the Lead remembers about these items (ADR-155).
        let memory = self
            .recall_text(
                lead.id,
                project,
                run,
                None,
                items
                    .iter()
                    .map(|it| it.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
            .await?;
        let task = format!("{memory}{task}");
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
