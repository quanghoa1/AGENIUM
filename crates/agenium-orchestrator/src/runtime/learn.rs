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
        let ep = Episode {
            source: format!("ticket:{}", t.key),
            project: Some(project.to_string()),
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
        let provider = self.providers.provider(&agent.model.provider)?;
        let model = PipelineModel {
            model: agent.model.model.clone(),
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
