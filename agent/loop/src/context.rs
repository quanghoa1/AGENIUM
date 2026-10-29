//! Keeping a run inside its context window: old tool results cleared, the conversation compacted into notes, the
//! checkpoint at the end.

use super::*;

/// What an image counts for in the context estimates (about 1 500 tokens: a screenshot as the providers scale it).
pub(super) const IMAGE_CHARS: usize = 6_000;

/// Share of the context window (percent) from which old tool results are cleared, without a model call.
pub(super) const CLEAR_AT_PERCENT: u64 = 60;

/// Share from which the run's conversation is replaced by the model's notes (or the run checkpoints).
pub(super) const COMPACT_AT_PERCENT: u64 = 85;

/// The most recent steps kept verbatim after a compaction, as a share of the window (ADR-113; after DeepSeek's
/// harness, which keeps a recent tail instead of summarising everything).
pub(super) const RETAIN_PERCENT: u64 = 15;

/// Tokens kept free for the notes when the run's whole prefix is sent for them (ADR-113).
pub(super) const NOTES_ROOM: u64 = 4_096;

/// Tool-result messages at the end of the conversation that are never cleared.
pub(super) const KEEP_RESULTS: usize = 4;

/// A tool result shorter than this (characters) is never cleared.
pub(super) const CLEAR_MIN_CHARS: usize = 400;

pub(super) const CLEARED: &str = "[cleared to save context";

pub(super) const COMPACT: &str = "The context is nearly full: the conversation of this run up to here will be replaced by your notes \
(the most recent steps after it are kept as they are). \
Write them now, without calling tools, under these headings:\n\
## Goal\n## Constraints and preferences\n## Progress (Done / In progress / Blocked)\n## Key decisions\n\
## Next steps\n## Critical context (exact names, values, errors and commands you still need)\n\
If notes from an earlier compaction are above, update them: keep what still holds, move finished items to Done, add \
what is new. The files you read and changed are listed for you; do not list them.";

pub(super) const COMPACTED: &str =
    "[Context compacted: the older part of this run's conversation was replaced by your notes below; the most recent \
steps follow unchanged. Changes you made to files are still there. Continue from the next step.]";

/// Characters of a tool result shown to the model that writes compaction notes.
pub(super) const NOTES_RESULT_CHARS: usize = 2000;

/// Tools whose results stay when old results are cleared (Anthropic's context editing `exclude_tools`): what a skill
/// says and which MCP tools a search found are used for the whole run, and clearing them only makes the agent ask
/// again (ADR-142).
const KEEP_RESULTS_OF: [&str; 2] = ["load_skill", "tool_search"];

/// Clears the content of long tool results, all but the last `keep` tool-result messages. Unless `force`, only when
/// together they free at least a tenth of the window (fewer edits keep the prompt cache). Returns how many.
pub(super) fn clear_old_results(
    messages: &mut [Message],
    window: u64,
    keep: usize,
    force: bool,
) -> usize {
    let with_results: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.parts.iter().any(|p| matches!(p, Part::ToolResult { .. })))
        .map(|(i, _)| i)
        .collect();
    let old = &with_results[..with_results.len().saturating_sub(keep)];
    let clearable = |p: &Part| match p {
        Part::ToolResult { content, name, .. } => {
            content.len() >= CLEAR_MIN_CHARS
                && !content.starts_with(CLEARED)
                && !KEEP_RESULTS_OF.contains(&name.as_str())
        }
        Part::Image { .. } => true,
        _ => false,
    };
    let chars: usize = old
        .iter()
        .flat_map(|&i| messages[i].parts.iter())
        .filter(|p| clearable(p))
        .map(|p| match p {
            Part::ToolResult { content, .. } => content.len(),
            Part::Image { .. } => IMAGE_CHARS,
            _ => 0,
        })
        .sum();
    // About four characters a token.
    if chars == 0 || (!force && (chars as u64) / 4 < window / 10) {
        return 0;
    }
    let mut n = 0;
    let mut first = None;
    for &i in old {
        for p in &mut messages[i].parts {
            if clearable(p) {
                if let Part::ToolResult { content, .. } = p {
                    *content = format!(
                        "{CLEARED}: {} characters; call the tool again if you need it]",
                        content.chars().count()
                    );
                } else {
                    *p = Part::Text {
                        text: format!("{CLEARED}: an image; view it again if you need it]"),
                    };
                }
                n += 1;
                first.get_or_insert(i);
            }
        }
    }
    // A thinking block is valid only after the history it was written on: those after an edited result go (Claude
    // rejects or drops them otherwise; ADR-113).
    if let Some(i) = first {
        strip_thinking(&mut messages[i + 1..]);
    }
    n
}

/// Removes the thinking blocks of `messages` (their text and tool calls stay).
pub(super) fn strip_thinking(messages: &mut [Message]) {
    for m in messages {
        m.parts.retain(|p| !matches!(p, Part::Thinking { .. }));
    }
}

/// Where the recent tail kept verbatim after a compaction starts: the earliest assistant turn from which the rest of
/// the run fits in `budget` characters, so a tool call stays with its results. The older part keeps at least one
/// exchange (else everything is summarised: `messages.len()`).
pub(super) fn tail_start(messages: &[Message], start_len: usize, budget: usize) -> usize {
    let chars = |m: &Message| -> usize {
        m.parts
            .iter()
            .map(|p| match p {
                Part::Text { text } | Part::Thinking { text, .. } => text.len(),
                Part::ToolCall { args, .. } => args.to_string().len(),
                Part::ToolResult { content, .. } => content.len(),
                Part::Image { .. } => IMAGE_CHARS,
            })
            .sum()
    };
    let mut size = 0;
    let mut start = messages.len();
    for i in (start_len..messages.len()).rev() {
        size += chars(&messages[i]);
        if size > budget {
            break;
        }
        if messages[i].role == Role::Assistant && i >= start_len + 2 {
            start = i;
        }
    }
    start
}

/// A gateway that answers an oversized request with a 400 or 413 and no error text (OpenCode Go with DeepSeek,
/// 2026-10-26: `{"model": "…"}` for a request of about a million tokens, while 500 000 were accepted). Only a request
/// estimated above 100 000 tokens counts, so a bare 400 on an ordinary request is never taken for an overflow.
pub(super) fn bare_overflow(e: &agenium_llm::LlmError, messages: &[Message]) -> bool {
    let agenium_llm::LlmError::Http {
        status: 400 | 413,
        message,
        ..
    } = e
    else {
        return false;
    };
    let wordless = !message.to_ascii_lowercase().contains("error") && message.len() < 200;
    let chars: usize = messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .map(|p| match p {
            Part::Text { text } => text.len(),
            Part::ToolResult { content, .. } => content.len(),
            Part::ToolCall { args, .. } => args.to_string().len(),
            _ => 0,
        })
        .sum();
    wordless && chars / 4 > 100_000
}

/// The files the replaced conversation read and changed, by its tool calls and by the lists of an earlier
/// compaction note it starts with (so they carry over). Changed files are not listed as read too.
pub(super) fn files_touched(messages: &[Message]) -> (Vec<String>, Vec<String>) {
    let mut read = std::collections::BTreeSet::new();
    let mut changed = std::collections::BTreeSet::new();
    let block = |text: &str, tag: &str| -> Vec<String> {
        let (open, close) = (format!("<{tag}>\n"), format!("\n</{tag}>"));
        text.find(&open)
            .and_then(|i| {
                let rest = &text[i + open.len()..];
                rest.find(&close)
                    .map(|j| rest[..j].lines().map(str::to_owned).collect())
            })
            .unwrap_or_default()
    };
    for m in messages {
        let t = m.text();
        if t.starts_with("[Context compacted") {
            read.extend(block(&t, "read-files"));
            changed.extend(block(&t, "modified-files"));
        }
        for c in m.tool_calls() {
            let Some(path) = c.args["path"].as_str() else {
                continue;
            };
            match c.name.as_str() {
                "read_file" => {
                    read.insert(path.to_owned());
                }
                "write_file" | "edit_file" | "multi_edit" => {
                    changed.insert(path.to_owned());
                }
                _ => {}
            }
        }
    }
    let read = read.into_iter().filter(|p| !changed.contains(p)).collect();
    (read, changed.into_iter().collect())
}

pub(super) fn usage_window(u: &Usage) -> u64 {
    u.total_input() + u.output
}

impl Run<'_> {
    /// A request for notes (compaction, checkpoint): the run's exact prefix with the tools listed but not callable,
    /// so the prompt cache serves everything before the final instruction (ADR-113).
    pub(super) fn notes_request(&self, order: &WorkOrder, messages: &[Message]) -> ChatRequest {
        ChatRequest {
            tools_off: true,
            ..self.request(order, messages, true)
        }
    }

    /// Asks for notes on the older part of the run's conversation and replaces that part with them, keeping the most
    /// recent steps verbatim; the run goes on. `Ok(false)` when the model wrote no notes.
    pub(super) async fn compact(
        &self,
        order: &WorkOrder,
        messages: &mut Vec<Message>,
        start_len: usize,
        usage: &mut Usage,
        overflowed: bool,
    ) -> Result<bool, Outcome> {
        // The recent tail stays as it is; the notes cover what comes before it. That older part is a prefix of the
        // run's last request, so the prompt cache serves it (ADR-113). After the provider said the request does not
        // fit, or if this one does not, a smaller request.
        let split = tail_start(
            messages,
            start_len,
            (self.cfg.context_window * RETAIN_PERCENT / 100 * 4) as usize,
        );
        let mut whole = messages[..split].to_vec();
        whole.push(Message::user_text(COMPACT));
        let first = if overflowed {
            None
        } else {
            match self.call(&self.notes_request(order, &whole)).await {
                Ok(r) => Some(r),
                Err(CallError::Fatal(e))
                    if agenium_llm::is_context_overflow(&e) || bare_overflow(&e, &whole) =>
                {
                    None
                }
                Err(e) => return Err(call_outcome(e)),
            }
        };
        let r = match first {
            Some(r) => r,
            None => self.cut_notes(order, &messages[..split]).await?,
        };
        usage.add(&r.usage);
        self.observer.response(&r.usage);
        let notes = r.message.text();
        // Notes cut by the output limit are not notes to continue from.
        if notes.trim().is_empty() || r.stop == StopReason::MaxTokens {
            return Ok(false);
        }
        let (read, changed) = files_touched(&messages[start_len..split]);
        let mut tail = messages.split_off(split);
        // The kept steps' thinking was written on the full history; after the notes it would not match (ADR-113).
        strip_thinking(&mut tail);
        messages.truncate(start_len);
        let mut text = format!("{COMPACTED}\n\n{}", notes.trim());
        if !read.is_empty() {
            text.push_str(&format!(
                "\n\n<read-files>\n{}\n</read-files>",
                read.join("\n")
            ));
        }
        if !changed.is_empty() {
            text.push_str(&format!(
                "\n<modified-files>\n{}\n</modified-files>",
                changed.join("\n")
            ));
        }
        self.observer.control(
            "compact",
            &format!("{} characters of notes", notes.trim().len()),
        );
        let m = Message::user_text(text);
        self.observer.message(&m);
        messages.push(m);
        messages.extend(tail);
        tracing::info!("run context compacted");
        Ok(true)
    }

    /// The notes request when the whole prefix does not fit: no thinking, tool results cut short (ADR-103).
    pub(super) async fn cut_notes(
        &self,
        order: &WorkOrder,
        messages: &[Message],
    ) -> Result<agenium_llm::ChatResponse, Outcome> {
        let mut ask: Vec<Message> = messages
            .iter()
            .map(|m| Message {
                role: m.role,
                parts: m
                    .parts
                    .iter()
                    .filter(|p| !matches!(p, Part::Thinking { .. }))
                    .map(|p| match p {
                        Part::ToolResult {
                            call_id,
                            name,
                            content,
                            is_error,
                        } if content.chars().count() > NOTES_RESULT_CHARS => Part::ToolResult {
                            call_id: call_id.clone(),
                            name: name.clone(),
                            content: format!(
                                "{}…[cut]",
                                content.chars().take(NOTES_RESULT_CHARS).collect::<String>()
                            ),
                            is_error: *is_error,
                        },
                        other => other.clone(),
                    })
                    .collect(),
            })
            .collect();
        ask.push(Message::user_text(COMPACT));
        self.turn(&self.notes_request(order, &ask)).await
    }

    /// Asks for a progress note without tools; the next run resumes from it.
    pub(super) async fn checkpoint(
        &self,
        order: &WorkOrder,
        messages: &mut Vec<Message>,
        usage: &mut Usage,
        reason: &str,
    ) -> Outcome {
        let m = Message::user_text(CHECKPOINT);
        self.observer.message(&m);
        messages.push(m);
        match self.turn(&self.notes_request(order, messages)).await {
            Ok(r) => {
                usage.add(&r.usage);
                self.observer.response(&r.usage);
                let summary = r.message.text();
                self.observer.message(&r.message);
                messages.push(r.message);
                Outcome::Checkpoint {
                    summary,
                    reason: reason.into(),
                }
            }
            Err(o) => o,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agenium_llm::Role;

    fn result(name: &str, chars: usize) -> Message {
        Message {
            role: Role::User,
            parts: vec![Part::ToolResult {
                call_id: name.into(),
                name: name.into(),
                content: "y".repeat(chars),
                is_error: false,
            }],
        }
    }

    #[test]
    fn a_skill_and_a_tool_search_stay_when_old_results_are_cleared() {
        let mut m = vec![
            result("load_skill", 9_000),
            result("tool_search", 9_000),
            result("read_file", 9_000),
            result("list_dir", 500),
        ];
        assert_eq!(clear_old_results(&mut m, 10_000, 1, true), 1);
        let text = |i: usize| match &m[i].parts[0] {
            Part::ToolResult { content, .. } => content.clone(),
            _ => unreachable!(),
        };
        assert!(!text(0).starts_with(CLEARED) && !text(1).starts_with(CLEARED));
        assert!(text(2).starts_with(CLEARED));
    }
}
