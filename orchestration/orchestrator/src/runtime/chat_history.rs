//! The Lead's chat history as a chat turn sees it (ADR-106). The conversation with the user is stored whole
//! (append-only); when it grows past `CHAT_BUDGET` estimated tokens, the older part is summarised once and the turn
//! sees `[Earlier conversation, summarised] …` followed by the recent messages, as pi does with its compaction
//! entries. The summary and where it ends are kept in the Lead's database; the next summary updates this one.
//! Both limits shrink with the Lead's model (ADR-155): a fifth of its context window at most, so a Lead moved from a
//! 1M-token model to a smaller one has its history summarised to fit; the summary is written by the Lead's own model
//! (API or CLI), piece by piece when the part to summarise is longer than that model reads at once.

use agenium_core::*;
use agenium_llm::{Message, Part, Role};
use agenium_store::AgentStore;

use super::Runtime;
use crate::Result;

/// Estimated tokens of history above which the older part is summarised (at most; a fifth of a smaller window).
pub const CHAT_BUDGET: usize = 50_000;
/// Estimated tokens of recent messages kept as they are (at most; 6 % of a smaller window).
pub const KEEP_RECENT: usize = 15_000;

/// The history budget and the recent part kept for a model with `window` tokens (0: not known).
pub fn budgets(window: u64) -> (usize, usize) {
    if window == 0 {
        return (CHAT_BUDGET, KEEP_RECENT);
    }
    let w = window as usize;
    (CHAT_BUDGET.min(w / 5), KEEP_RECENT.min(w * 6 / 100))
}
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
fn cut(messages: &[Message], keep: usize) -> Option<usize> {
    let mut acc = 0;
    let mut i = messages.len();
    while i > 0 && acc < keep {
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
    /// summary once it is too long. A failed summary leaves the history whole (and is tried at the next turn). Also
    /// what the summary's model call used, priced: the turn that needed it records it with its own.
    pub(crate) async fn chat_history(
        &self,
        lead: &agenium_store::core::agents::Agent,
        mem: &AgentStore,
        project: ProjectId,
    ) -> Result<(Vec<Message>, UsageSummary)> {
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
        // The summary's model call is recorded as a helper call of its own (`chat-summary`), priced there.
        let none = UsageSummary::default();
        let window = self.window_of(lead).await;
        let (budget, keep) = budgets(window);
        if current.iter().map(tokens).sum::<usize>() <= budget {
            return Ok((current, none));
        }
        let Some(rel) = cut(&all[from..], keep) else {
            return Ok((current, none));
        };
        let until = from + rel;
        // What the Lead's model reads at once: the part to summarise in pieces of at most two fifths of its window.
        let piece_chars = (window.max(32_000) * 40 / 100 * 4) as usize;
        let transcript = super::learn::chat_transcript(&all[from..until]);
        let mut pieces: Vec<String> = Vec::new();
        let mut rest = transcript.as_str();
        while !rest.is_empty() {
            let mut end = rest.len().min(piece_chars);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            // A piece ends at a line where it can.
            if end < rest.len() {
                if let Some(nl) = rest[..end].rfind('\n').filter(|n| *n > end / 2) {
                    end = nl + 1;
                }
            }
            pieces.push(rest[..end].to_owned());
            rest = &rest[end..];
        }
        let mut s = summary.clone();
        for (k, piece) in pieces.iter().enumerate() {
            let mut text = String::new();
            if !s.is_empty() {
                text.push_str(&format!("## Earlier summary\n{s}\n\n"));
            }
            text.push_str(&format!(
                "## Conversation to summarise (part {} of {})\n{piece}",
                k + 1,
                pieces.len()
            ));
            match self
                .side_chat(
                    "chat-summary",
                    None,
                    lead.backend,
                    &lead.model,
                    SUMMARY_RULES,
                    vec![Message::user_text(text)],
                    6000,
                )
                .await
            {
                Ok(a) if !a.text.trim().is_empty() => s = a.text.trim().to_owned(),
                Ok(_) => {
                    tracing::warn!(%project, "chat summary was empty; history kept whole");
                    return Ok((current, none));
                }
                Err(e) => {
                    tracing::warn!(%project, error = %e, "chat summary failed; history kept whole");
                    return Ok((current, none));
                }
            }
        }
        mem.set_meta(
            &mark,
            &serde_json::to_string(&(until, &s)).unwrap_or_default(),
        )?;
        tracing::info!(%project, summarised = until, pieces = pieces.len(), "the Lead's chat history was summarised");
        Ok((view(until, &s), none))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_history_budget_shrinks_with_the_window() {
        assert_eq!(budgets(0), (CHAT_BUDGET, KEEP_RECENT));
        assert_eq!(budgets(1_000_000), (CHAT_BUDGET, KEEP_RECENT));
        assert_eq!(budgets(256_000), (CHAT_BUDGET, KEEP_RECENT));
        assert_eq!(budgets(128_000), (25_600, 7_680));
        assert_eq!(budgets(32_000), (6_400, 1_920));
    }
}
