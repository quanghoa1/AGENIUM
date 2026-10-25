//! Memory pipeline trigger (decision 2): when a ticket becomes Closed or Reopened the assignee's own model turns the
//! episode (ticket, comments with the review feedback, its last work conversation) into memory operations. It runs
//! once per ticket state (marker in the agent database) and never fails the review that triggered it.

use agenium_core::*;
use agenium_llm::{Message, Part};
use agenium_memory::pipeline::{learn, Episode, LearnReport, PipelineModel};
use agenium_store::core::{comments, runs};

use super::Runtime;
use crate::Result;

/// Plain text of a conversation for extraction: text, tool calls (name + short args) and short results; thinking
/// blocks are never used (decision 13).
pub fn transcript(messages: &[Message]) -> String {
    let mut s = String::new();
    for m in messages {
        for p in &m.parts {
            match p {
                Part::Text { text } => s.push_str(&format!("[{:?}] {}\n", m.role, text.trim())),
                Part::ToolCall { name, args, .. } => {
                    let a: String = args.to_string().chars().take(300).collect();
                    s.push_str(&format!("[tool call] {name} {a}\n"));
                }
                Part::ToolResult {
                    name,
                    content,
                    is_error,
                    ..
                } => {
                    let c: String = content.chars().take(600).collect();
                    let tag = if *is_error {
                        "tool error"
                    } else {
                        "tool result"
                    };
                    s.push_str(&format!("[{tag}] {name}: {c}\n"));
                }
                Part::Thinking { .. } | Part::Image { .. } => {}
            }
        }
    }
    s
}

impl Runtime {
    /// Runs the memory pipeline for the ticket's assignee if the ticket is Closed or Reopened and this state was
    /// not learned from yet. Errors are logged on `log_run`.
    pub async fn learn_from_ticket(
        &self,
        project: ProjectId,
        key: &TicketKey,
        log_run: RunId,
    ) -> Result<Option<LearnReport>> {
        if !self.app.config().memory.learn {
            return Ok(None);
        }
        let t = self.app.ticket(project, key)?;
        if !matches!(t.status, TicketStatus::Closed | TicketStatus::Reopened) {
            return Ok(None);
        }
        let Some(agent_id) = t.assignee else {
            return Ok(None);
        };
        let agent = self.app.agent(agent_id)?;
        let mem = self.agent_memory(agent.id)?;
        let marker = format!("learned:{}:{}:{}", t.key, t.reopen_count, t.status);
        if mem.meta(&marker)?.is_some() {
            return Ok(None);
        }
        let all = self.app.store().read(|c| runs::for_ticket(c, t.id))?;
        let last_work = all
            .iter()
            .rev()
            .find(|r| r.agent == agent.id && r.ended_at.is_some() && !self.is_review_run(r));
        let conversation = match last_work {
            Some(r) => mem
                .load_messages(&r.id.to_string())?
                .into_iter()
                .filter_map(|v| serde_json::from_value::<Message>(v).ok())
                .collect(),
            None => vec![],
        };
        let cmts = self.app.store().read(|c| comments::for_ticket(c, t.id))?;
        let used: Vec<(i64, String)> = match last_work {
            Some(r) => mem
                .facts_used_in(&r.id.to_string())?
                .into_iter()
                .map(|f| (f.id, f.text))
                .collect(),
            None => vec![],
        };
        // Decision 35: what makes the episode worth a lesson.
        let mut signals = Vec::new();
        if t.status == TicketStatus::Reopened {
            signals.push(format!(
                "the review reopened the work (reopen #{})",
                t.reopen_count
            ));
        } else if t.reopen_count > 0 {
            signals.push(format!(
                "accepted after {} reopen(s): what changed between the rejected and the accepted work",
                t.reopen_count
            ));
        }
        if cmts.iter().any(
            |c| matches!(&c.body, CommentBody::System { text } if text.starts_with("Merge failed")),
        ) {
            signals.push("a merge or pre-merge check failed".into());
        }
        if cmts
            .iter()
            .any(|c| c.author == Author::User && matches!(c.body, CommentBody::Text { .. }))
        {
            signals.push("the user commented on the work".into());
        }
        let ep = Episode {
            signals,
            source: format!("ticket:{}", t.key),
            project: Some(project.to_string()),
            project_name: Some(self.app.project(project)?.name),
            reopened: t.status == TicketStatus::Reopened,
            run: last_work.map(|r| r.id.to_string()),
            used,
            outcome: match t.status {
                TicketStatus::Reopened => {
                    format!(
                        "Reopened by the review (reopen #{}): see the review comments",
                        t.reopen_count
                    )
                }
                _ => "Closed: the work was accepted".into(),
            },
            transcript: format!(
                "{}\n{}\n## Your work conversation\n{}",
                agenium_backend::prompt::ticket_text(&t, None),
                agenium_backend::prompt::comments_text(&cmts),
                transcript(&conversation)
            ),
        };
        // ADR-85: on routine work with nothing to reconcile, a cheap judge read can skip the costly extraction call.
        // Only when there are no signals, no memories were shown and the ticket was not reopened (so the reuse
        // bookkeeping below has nothing to do) and the judge is confident the episode holds nothing worth keeping.
        if ep.signals.is_empty()
            && ep.used.is_empty()
            && !ep.reopened
            && self.app.worth_remembering(&ep.transcript).await == Some(false)
        {
            tracing::info!(ticket = %t.key, "memory extraction skipped: the judge found nothing worth remembering");
            mem.set_meta(&marker, &chrono::Utc::now().to_rfc3339())?;
            let _ = self.app.run_log(
                log_run,
                "memory",
                serde_json::json!({ "skipped": "judge: nothing worth remembering" }),
            );
            return Ok(None);
        }
        // Extraction is an API call: a CLI agent uses the `[cli]` API fallback model for it (ADR-56).
        let (provider_id, model_id) = if agent.backend == BackendKind::Cli {
            let cfg = self.app.config().cli;
            if cfg.fallback_provider.is_empty() || cfg.fallback_model.is_empty() {
                tracing::info!(
                    agent = %agent.name,
                    "memory pipeline skipped: a CLI agent needs [cli] fallback_provider/fallback_model for extraction"
                );
                return Ok(None);
            }
            (cfg.fallback_provider, cfg.fallback_model)
        } else {
            (agent.model.provider.clone(), agent.model.model.clone())
        };
        let provider = self.providers.provider(&provider_id)?;
        let model = PipelineModel {
            model: model_id,
            effort: Effort::Low,
            max_tokens: 4000,
            retry: self.retry,
        };
        let redactor = self.app.redactor().clone();
        let clean = move |text: &str| -> Option<String> {
            let r = redactor.redact(text);
            (r == text && agenium_secrets::scan_text(text).is_empty()).then_some(r)
        };
        match learn(
            provider.as_ref(),
            &model,
            &mem,
            self.embedder.as_ref(),
            &ep,
            &clean,
        )
        .await
        {
            Ok(rep) => {
                mem.set_meta(&marker, &chrono::Utc::now().to_rfc3339())?;
                let _ = self.app.run_log(
                    log_run,
                    "memory",
                    serde_json::to_value(&rep).unwrap_or_default(),
                );
                // ADR-87: promote this agent's now-generalized lessons into the shared pool.
                if let Err(e) = self.share_lessons(agent_id, &mem) {
                    tracing::warn!(error = %e, "sharing generalized lessons failed");
                }
                // ADR-91: a skill the agent created is flagged for the user once its lesson stops holding (anti-drift).
                if let Err(e) = self.app.audit_created_skills(agent_id) {
                    tracing::warn!(error = %e, "auditing created skills failed");
                }
                Ok(Some(rep))
            }
            Err(e) => {
                tracing::warn!(ticket = %t.key, error = %e, "memory pipeline failed");
                let _ = self.app.run_log(
                    log_run,
                    "memory",
                    serde_json::json!({ "error": e.to_string() }),
                );
                Ok(None)
            }
        }
    }

    /// Promotes `agent`'s generalized lessons into the shared pool (ADR-87): a live, trusted Lesson that held in two
    /// or more distinct contexts and is not scoped to one project. The pool copy drops the project, keeps scope and cue,
    /// and caps confidence; pool recall ranks it below the recalling agent's own memory. Deduped by text, so repeated
    /// runs do not pile up copies. No-op when `[memory] share` is off.
    fn share_lessons(
        &self,
        agent: AgentId,
        mem: &std::sync::Arc<agenium_store::AgentStore>,
    ) -> Result<usize> {
        if !self.app.config().memory.share {
            return Ok(0);
        }
        let pool = self.app.pool_memory()?;
        let norm = |s: &str| s.trim().to_lowercase();
        let have: std::collections::HashSet<String> = pool
            .live(10_000)?
            .into_iter()
            .map(|f| norm(&f.text))
            .collect();
        let mut n = 0;
        for f in mem.live(10_000)? {
            let project_scoped = f
                .scope
                .as_object()
                .is_some_and(|o| o.contains_key("project"));
            let general = f.kind == agenium_store::FactKind::Lesson
                && !f.untrusted
                && f.validated.len() >= crate::skill_overlay::MIN_CONTEXTS
                && !project_scoped;
            if !general || have.contains(&norm(&f.text)) {
                continue;
            }
            let nf = agenium_store::NewFact {
                text: f.text.clone(),
                kind: agenium_store::FactKind::Lesson,
                project: None,
                scope: f.scope.clone(),
                cue: f.cue.clone(),
                evidence: None,
                source: format!("shared:{agent}"),
                untrusted: false,
                confidence: f.confidence.min(0.6),
            };
            let vector = self
                .embedder
                .embed(std::slice::from_ref(&f.text))
                .ok()
                .and_then(|mut v| v.pop());
            pool.add(&nf, vector.as_deref())?;
            n += 1;
        }
        Ok(n)
    }

    fn is_review_run(&self, r: &runs::Run) -> bool {
        self.app
            .store()
            .read(|c| runs::log_entries(c, r.id))
            .map(|l| {
                l.iter()
                    .any(|e| e.kind == "run" && e.data["kind"] == "review")
            })
            .unwrap_or(false)
    }
}
