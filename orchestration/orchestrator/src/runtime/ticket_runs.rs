//! Ticket runs: the assignee's work run on a ready ticket and the Lead's review run on a Resolved one (and their
//! outcome handling). Split out of `runtime/mod.rs`.

use super::*;

impl Runtime {
    /// Runs the assignee on a ready ticket (start -> loop -> outcome handling).
    pub async fn run_ticket(&self, project: ProjectId, key: &TicketKey) -> Result<RunSummary> {
        let mut opened = None;
        let r = self.work_run(project, key, &mut opened).await;
        self.end_on_error(opened, r)
    }

    async fn work_run(
        &self,
        project: ProjectId,
        key: &TicketKey,
        opened: &mut Option<RunId>,
    ) -> Result<RunSummary> {
        // Decision 19: changes made to the worktree outside Agenium are undone before the agent continues.
        if let Err(e) = self.app.guard_worktree(project, key) {
            tracing::warn!(ticket = %key, error = %e, "worktree guard failed");
        }
        let t = self.app.ticket(project, key)?;
        let agent_id = t
            .assignee
            .ok_or_else(|| OrchError::Invalid(format!("{key} has no assignee")))?;
        let agent = self.app.agent(agent_id)?;
        let team_id = agent
            .team
            .ok_or_else(|| OrchError::Invalid(format!("{} is in no team", agent.name)))?;
        let team = self
            .app
            .store()
            .read(|c| agenium_store::core::agents::get_team(c, team_id))?;
        // The worktree is made when the ticket is assigned (its git operation is retried every round): without it no
        // run starts.
        let ws = self
            .app
            .store()
            .read(|c| workspaces::get(c, t.id))?
            .filter(|w| !w.removed)
            .ok_or_else(|| OrchError::Invalid(format!("{key} has no worktree")))?;
        let root = PathBuf::from(&ws.worktree);
        let start = self
            .app
            .start_run(project, key, &agent.model.provider, &agent.model.model)?;
        *opened = Some(start.run);
        if !start.continuation {
            // A dependent ticket starts from the main that contains its dependencies' merged work.
            if let Err(e) = self.app.catch_up_worktree(project, key) {
                tracing::warn!(ticket = %key, error = %e, "worktree not brought up to date");
            }
        }
        let t = self.app.ticket(project, key)?;
        let mem = self.agent_memory(agent.id)?;
        let (prev, note, prev_start) = if start.continuation {
            self.previous(agent.id, t.id, &mem)?
        } else {
            (vec![], None, None)
        };
        // Who worked on the ticket before this agent (handed over when it ran out of quota or was reassigned): their
        // conversation is theirs, their changes are in the worktree.
        let mut before: Vec<String> = Vec::new();
        for r in self.app.store().read(|c| runs::for_ticket(c, t.id))? {
            if r.agent != agent.id {
                let name = self.app.agent(r.agent).map(|a| a.name).unwrap_or_default();
                if !name.is_empty() && !before.contains(&name) {
                    before.push(name);
                }
            }
        }
        let solo = team.solo;
        // ADR-87: also recall from the shared pool of generalized lessons (ranked below the agent's own memory).
        let pool = if self.app.config().memory.share {
            self.app.pool_memory().ok()
        } else {
            None
        };
        let retriever = MemoryRetriever::new(mem.clone(), self.embedder.clone()).with_pool(pool);
        let query = format!(
            "{}\n{}\n{}",
            t.title,
            t.description,
            t.acceptance
                .iter()
                .map(|a| a.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
        let recalled = retriever
            .recall_in(
                &query,
                &agenium_memory::retrieve::RecallContext {
                    project: Some(project.to_string()),
                    scope: serde_json::json!({ "project": self.app.project(project)?.name }),
                },
                self.memory_k,
            )
            .unwrap_or_default();
        retriever
            .record(&start.run.to_string(), Some(t.key.as_str()), &recalled)
            .ok();
        // The run page shows what the agent was reminded of: the memories its first message carries.
        if prev.is_empty() {
            let shown: Vec<serde_json::Value> = recalled
                .iter()
                .map(|r| serde_json::json!({ "id": r.id, "kind": r.kind, "text": r.text, "shared": r.shared, "exact": r.exact }))
                .collect();
            let _ = self.app.run_log(
                start.run,
                "memory",
                serde_json::json!({ "recalled": shown }),
            );
        }
        let all_comments = self.app.store().read(|c| comments::for_ticket(c, t.id))?;
        let requirements = self.app.ticket_requirements(project, &t)?;
        let guidance = self.app.project_guidance(project)?;
        let soul = self.soul_and_role(&agent);
        // ADR-82: on a fresh run, the judge may pick the best-fitting skill so the worker loads the right one
        // without reading them all. Fail-open: `None` unless the judge is on and confident.
        let skill_hint = if start.continuation {
            None
        } else {
            let skills: Vec<(String, String)> = self
                .skills(&agent)
                .skills
                .iter()
                .map(|s| (s.meta.name.clone(), s.meta.description.clone()))
                .collect();
            let hint = self.app.skill_hint(&t, &skills).await;
            if let Some(h) = &hint {
                let _ = self.app.run_log(
                    start.run,
                    "judge",
                    serde_json::json!({ "use": "skill_hint", "text": h.trim() }),
                );
            }
            hint
        };
        // ADR-91: the skill-writing tools (propose_skill_update, propose_new_skill) are offered only when the agent
        // holds a lesson that may become skill knowledge — live, trusted, held in two distinct contexts. Without one
        // they are withheld, so an agent never writes skills from unproven experience.
        let ready: Vec<i64> = mem
            .live(10_000)?
            .into_iter()
            .filter(|f| {
                f.kind == agenium_store::FactKind::Lesson
                    && !f.untrusted
                    && f.validated.len() >= crate::skill_overlay::MIN_CONTEXTS
            })
            .map(|f| f.id)
            .collect();
        let skill_tools_off: &[ProtocolTool] = if ready.is_empty() {
            &[
                ProtocolTool::ProposeSkillUpdate,
                ProtocolTool::ProposeNewSkill,
            ]
        } else {
            &[]
        };
        let ready_hint = (!ready.is_empty() && !start.continuation).then(|| {
            let ids: Vec<String> = ready.iter().take(8).map(|id| format!("#{id}")).collect();
            format!(
                "\nLessons that held in two or more contexts: {}. When the ticket's work is done and before you call \
                 submit_report (never instead of the work): if one is a reusable procedure that a skill of yours covers, \
                 improve that skill with propose_skill_update; only if no skill covers it, write it down as a new skill with \
                 propose_new_skill (every section filled, a Proof that demands evidence). If none is a reusable procedure, \
                 do nothing.\n",
                ids.join(", ")
            )
        });
        let started = Instant::now();
        let builder = |ctx: &ToolCtx, specs: &[ToolSpec]| -> Result<WorkOrder> {
            let (manifest, _) = self.manifest(&agent, solo, ctx, specs, None, &ctx.fetch_allow);
            let mut system = vec![
                SystemBlock {
                    text: prompt::system_rules(solo),
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
            ];
            // The project's guidance is stable across tickets: it goes before the per-run manifest (ADR-113).
            if let Some(g) = &guidance {
                system.insert(
                    2,
                    SystemBlock {
                        text: g.clone(),
                        cache: true,
                    },
                );
            }
            let mut messages = prev.clone();
            if messages.is_empty() {
                let facts: Vec<String> = recalled
                    .iter()
                    .map(agenium_memory::retrieve::label)
                    .collect();
                let mut first = format!(
                    "{}{}{}\n",
                    prompt::memory_text(&facts),
                    prompt::ticket_text(&t, None),
                    requirements
                );
                first.push_str(&prompt::comments_text(&all_comments));
                first.push_str(&self.user_decisions(project, &t.key)?);
                if let Some(n) = &note {
                    first.push_str(&format!("\n## Notes from your previous run\n{n}\n"));
                }
                if !before.is_empty() {
                    first.push_str(&format!(
                        "\n## Work so far\n{} worked on this ticket before you (the comments above say how far and why they stopped). Their changes are in your worktree: look at them first (`git status`, `git diff`) and go on from them; do not start over.\n",
                        before.join(", ")
                    ));
                }
                first.push_str(&format!("\nYour worktree is {} on branch agenium/{}. Do the work, then call submit_report.", ctx.root.display(), t.key));
                if let Some(h) = &skill_hint {
                    first.push_str(h);
                }
                if let Some(h) = &ready_hint {
                    first.push_str(h);
                }
                messages.push(Message::user_text(first));
            } else {
                // Everything posted since the previous run started (a comment posted while it ran never reached it),
                // except the agent's own: those are in its conversation already.
                let newer: Vec<Comment> = all_comments
                    .iter()
                    .filter(|c| {
                        c.author != Author::Agent(agent.id)
                            && prev_start
                                .as_deref()
                                .is_none_or(|s| c.created_at.as_str() > s)
                    })
                    .cloned()
                    .collect();
                let mut text = String::from("Your run resumes.");
                if !newer.is_empty() {
                    text.push_str(&format!(
                        " New on the ticket since your last step:\n{}",
                        prompt::comments_text(&newer)
                    ));
                }
                text.push_str("\nContinue the work; call submit_report when every acceptance criterion is met.");
                messages.push(Message::user_text(text));
            }
            Ok(WorkOrder { system, messages })
        };
        let res = self
            .execute(
                RunKind::Work,
                project,
                &agent,
                solo,
                start.run,
                Some(&t),
                &root,
                builder,
                agenium_tools::work_tools(),
                skill_tools_off,
            )
            .await;
        let (report, cost) = res?;
        self.finish(
            project,
            &t,
            &agent,
            start.run,
            "work",
            report,
            cost,
            started.elapsed().as_millis() as u64,
        )
    }

    /// Lead review of a Resolved ticket: report + diff -> review_ticket (approve merges) -> close_ticket.
    pub async fn review(&self, project: ProjectId, key: &TicketKey) -> Result<RunSummary> {
        let mut opened = None;
        let r = self.review_run(project, key, &mut opened).await;
        self.end_on_error(opened, r)
    }

    async fn review_run(
        &self,
        project: ProjectId,
        key: &TicketKey,
        opened: &mut Option<RunId>,
    ) -> Result<RunSummary> {
        if let Err(e) = self.app.guard_worktree(project, key) {
            tracing::warn!(ticket = %key, error = %e, "worktree guard failed");
        }
        let t = self.app.ticket(project, key)?;
        let (team, _) = self
            .app
            .team_of_project(project)?
            .ok_or_else(|| OrchError::Invalid("no team".into()))?;
        let lead = self.app.agent(
            team.lead
                .ok_or_else(|| OrchError::Invalid("the team has no Lead".into()))?,
        )?;
        let ws = self
            .app
            .store()
            .read(|c| workspaces::get(c, t.id))?
            .filter(|w| !w.removed)
            .ok_or_else(|| OrchError::Invalid(format!("{key} has no worktree")))?;
        let root = PathBuf::from(&ws.worktree);
        let repo = self.app.repo(&self.app.project(project)?);
        // A diff that cannot be read is said so: shown empty, it would read as "no code changes".
        let diff = match repo.diff(&root, &ws.base_sha) {
            Ok(d) if d.trim().is_empty() => "(no code changes)".to_owned(),
            Ok(d) => agenium_tools::text::truncate(&d, 30_000, 600).0,
            Err(e) => {
                format!("(the diff could not be read: {e}; read the changes in the worktree)")
            }
        };
        let run = self
            .app
            .store()
            .commit(None, "system", |w| -> Result<RunId> {
                let id = RunId::new();
                runs::insert(
                    w.tx,
                    &runs::Run {
                        id,
                        ticket: Some(t.id),
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
                    ticket: Some(t.id),
                })?;
                Ok(id)
            })?
            .value;
        *opened = Some(run);
        let all_comments = self.app.store().read(|c| comments::for_ticket(c, t.id))?;
        let requirements = self.app.ticket_requirements(project, &t)?;
        let guidance = self.app.project_guidance(project)?;
        // Optional judge: an independent first read of the report's evidence (ADR-75), token-light.
        let report_text = all_comments.iter().rev().find_map(|c| match &c.body {
            CommentBody::Report(r) => Some(crate::judge::report_text(r)),
            _ => None,
        });
        let evidence = match &report_text {
            Some(rep) => self.app.review_evidence(&t, rep).await.unwrap_or_default(),
            None => String::new(),
        };
        if !evidence.is_empty() {
            let _ = self.app.run_log(
                run,
                "judge",
                serde_json::json!({ "use": "review_assist", "text": evidence.trim() }),
            );
        }
        let soul = self.soul_and_role(&lead);
        let solo = team.solo;
        let started = Instant::now();
        // The reviewer judges against what the user decided, also on other tickets (ADR-120): a reviewer that
        // did not know "no Python here, use PowerShell" doubted the substitute and wanted to ask again.
        let decisions = self.user_decisions(project, &t.key)?;
        let builder = |ctx: &ToolCtx, specs: &[ToolSpec]| -> Result<WorkOrder> {
            let (manifest, run_context) =
                self.manifest(&lead, solo, ctx, specs, None, &ctx.fetch_allow);
            let mut system = vec![
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
            ];
            // The project's guidance is stable across tickets: it goes before the per-run manifest (ADR-113).
            if let Some(g) = &guidance {
                system.insert(
                    2,
                    SystemBlock {
                        text: g.clone(),
                        cache: true,
                    },
                );
            }
            let text = format!(
                "Review task for {key}. Compare the work with every acceptance criterion using the report, the diff and, if needed, the read-only tools on the worktree. \
                 Treat the report as a claim: check each criterion's evidence yourself (the test, the changed lines, the file) before you count it as met.\n\
                 Then call review_ticket: Approve if every criterion is met (one the report left Partial or NotMet needs ac_feedback saying how you verified it), otherwise Reopen with feedback per unmet criterion (what is missing and how you will check it). \
                 Approving a code ticket runs the merge queue at once and tells you the result. After a successful merge (or for a ticket without code), call close_ticket with a reason.\n\n{}{}{}\n{}\n## Diff (branch agenium/{key} against its base)\n```diff\n{}\n```",
                prompt::ticket_text(&t, None),
                requirements,
                evidence,
                format_args!("{}{decisions}", prompt::comments_text(&all_comments)),
                diff
            );
            Ok(WorkOrder {
                system,
                messages: vec![Message::user_text(format!("{run_context}\n\n{text}"))],
            })
        };
        let read_only: Vec<Arc<dyn Tool>> = agenium_tools::work_tools()
            .into_iter()
            .filter(|t| t.read_only() && t.name() != "fetch")
            .collect();
        let res = self
            .execute(
                RunKind::Review,
                project,
                &lead,
                solo,
                run,
                Some(&t),
                &root,
                builder,
                read_only,
                &[
                    ProtocolTool::ProposeSkillUpdate,
                    ProtocolTool::ProposeNewSkill,
                ],
            )
            .await;
        let (report, cost) = res?;
        let summary = self.finish(
            project,
            &t,
            &lead,
            run,
            "review",
            report,
            cost,
            started.elapsed().as_millis() as u64,
        )?;
        // The assignee learns from the outcome (decision 2); a failure here never fails the review.
        if let Err(e) = self.learn_from_ticket(project, key, run).await {
            tracing::warn!(ticket = %key, error = %e, "memory pipeline skipped");
        }
        Ok(summary)
    }
}
