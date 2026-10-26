//! The Lead's chat history as a chat turn sees it (ADR-106). The conversation with the user is stored whole
//! (append-only); when it grows past `CHAT_BUDGET` estimated tokens, the older part is summarised once and the turn
//! sees `[Earlier conversation, summarised] …` followed by the recent messages, as pi does with its compaction
//! entries. The summary and where it ends are kept in the Lead's database; the next summary updates this one.

use agenium_core::*;
use agenium_llm::{chat_with_retry, ChatRequest, Message, Part, Role, StopReason, SystemBlock};
use agenium_store::AgentStore;

use super::Runtime;
use crate::Result;

/// Estimated tokens of history above which the older part is summarised.
pub const CHAT_BUDGET: usize = 50_000;
/// Estimated tokens of recent messages kept as they are.
pub const KEEP_RECENT: usize = 15_000;
pub const SUMMARISED: &str = "[Earlier conversation, summarised]";

const SUMMARY_RULES: &str = "You are the Lead agent. Summarise the earlier part of your conversation with the user \
for your own later use, under these headings:\n## Goal\n## Agreements and decisions\n## Preferences the user stated\n\
## Open questions\n## State of the work (specs, plans, tickets)\nKeep exact names, numbers and wording that matter. If \
an earlier summary is included, update it: keep what still holds, add what is new. Answer with the summary only.";

fn tokens(m: &Message) -> usize {
    m.parts
        .iter()
        .map(|p| match p {
            Part::Text { text } => text.len(),
            Part::ToolResult { content, .. } => content.len(),
            Part::ToolCall { args, .. } => args.to_string().len() + 20,
            Part::Thinking { text, .. } => text.len(),
            Part::Image { .. } => 4800,
        })
        .sum::<usize>()
        / 4
}

/// The first message of the kept tail: walk back until `KEEP_RECENT` is reached, then forward to a message the user
/// wrote (never between a tool call and its result). `None` when no such cut exists.
fn cut(messages: &[Message]) -> Option<usize> {
    let mut acc = 0;
    let mut i = messages.len();
    while i > 0 && acc < KEEP_RECENT {
        i -= 1;
        acc += tokens(&messages[i]);
    }
    (i..messages.len()).find(|&j| {
        j > 0
            && messages[j].role == Role::User
            && messages[j]
                .parts
                .iter()
                .any(|p| matches!(p, Part::Text { .. }))
            && !messages[j]
                .parts
                .iter()
                .any(|p| matches!(p, Part::ToolResult { .. }))
    })
}

impl Runtime {
    /// The history a chat turn of `lead` in `project` sends: the stored conversation, its older part replaced by a
    /// summary once it is too long. A failed summary leaves the history whole (and is tried at the next turn).
    pub(crate) async fn chat_history(
        &self,
        lead: &agenium_store::core::agents::Agent,
        mem: &AgentStore,
        project: ProjectId,
    ) -> Result<Vec<Message>> {
        let all: Vec<Message> = mem
            .load_messages(&super::learn::chat_session(project))?
            .into_iter()
            .filter_map(|v| serde_json::from_value(v).ok())
            .collect();
        let mark = format!("chat-summary:{project}");
        let (from, summary) = mem
            .meta(&mark)?
            .and_then(|v| serde_json::from_str::<(usize, String)>(&v).ok())
            .filter(|(n, _)| *n <= all.len())
            .unwrap_or((0, String::new()));
        let view = |from: usize, summary: &str| -> Vec<Message> {
            let mut v = Vec::new();
            if !summary.is_empty() {
                v.push(Message::user_text(format!("{SUMMARISED}\n{summary}")));
                // Their thinking was written on the whole conversation: after the summary it would not match (ADR-113).
                v.extend(all[from..].iter().cloned().map(|mut m| {
                    m.parts.retain(|p| !matches!(p, Part::Thinking { .. }));
                    m
                }));
            } else {
                v.extend(all[from..].iter().cloned());
            }
            v
        };
        let current = view(from, &summary);
        if current.iter().map(tokens).sum::<usize>() <= CHAT_BUDGET {
            return Ok(current);
        }
        let Some(rel) = cut(&all[from..]) else {
            return Ok(current);
        };
        let until = from + rel;
        let Some((provider, model)) = self.side_model(lead) else {
            return Ok(current);
        };
        let mut text = String::new();
        if !summary.is_empty() {
            text.push_str(&format!("## Earlier summary\n{summary}\n\n"));
        }
        text.push_str(&format!(
            "## Conversation to summarise\n{}",
            super::learn::chat_transcript(&all[from..until])
        ));
        let req = ChatRequest {
            model,
            system: vec![SystemBlock {
                text: SUMMARY_RULES.into(),
                cache: false,
            }],
            messages: vec![Message::user_text(text)],
            tools: vec![],
            effort: Effort::LOW,
            max_tokens: 6000,
            cache_key: None,
            long_cache: false,
            tools_off: false,
        };
        match chat_with_retry(provider.as_ref(), &req, &self.retry, &|_| {}, |_, _, _| {}).await {
            Ok(r) if r.stop != StopReason::MaxTokens && !r.message.text().trim().is_empty() => {
                let s = r.message.text().trim().to_owned();
                mem.set_meta(
                    &mark,
                    &serde_json::to_string(&(until, &s)).unwrap_or_default(),
                )?;
                tracing::info!(%project, summarised = until, "the Lead's chat history was summarised");
                Ok(view(until, &s))
            }
            Ok(_) => {
                tracing::warn!(%project, "chat summary was empty or cut off; history kept whole");
                Ok(current)
            }
            Err(e) => {
                tracing::warn!(%project, error = %e, "chat summary failed; history kept whole");
                Ok(current)
            }
        }
    }
}
