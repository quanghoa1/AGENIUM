//! Fitting a conversation into the model's context window (ADR-155). A run that resumes a conversation written for a
//! larger window (the agent moved from a 1M-token model to a 256K one), or that grew past its own, has its older part
//! folded into notes before anything is sent: piece by piece, each piece small enough for this model, the notes of
//! the pieces before carried into the next (as opencode and Gemini CLI fold a history into a summary and keep the
//! recent turns). The most recent steps stay verbatim; the notes and the files read and changed take the older
//! part's place, and the run's stored conversation is replaced by the result so the next run resumes it as it is.

use super::*;

/// Characters per token in the estimates (the same rough ratio the loop uses for tool results).
const CHARS_PER_TOKEN: u64 = 4;

/// Share of what is left of the window (after the system prompt, the tools and the notes' room) that one piece of
/// the older conversation may take.
const PIECE_PERCENT: u64 = 60;

/// Characters of one tool call's arguments or one tool result shown in a piece.
const PIECE_PART_CHARS: usize = 3_000;

pub const FOLD: &str = "The conversation of this run is longer than this model's context window, so its older part is being \
folded into notes, one piece at a time. Below are your notes so far (empty for the first piece) and the next piece of \
the conversation, as a transcript. Write the notes again, now covering this piece too, without calling tools, under \
these headings:\n## Goal\n## Constraints and preferences\n## Progress (Done / In progress / Blocked)\n## Key decisions\n\
## Next steps\n## Critical context (exact names, values, errors and commands you still need)\n\
Keep what still holds, move finished items to Done, add what is new. The files read and changed are listed for you; \
do not list them.";

/// Characters of a message as the estimates count them.
pub(super) fn message_chars(m: &Message) -> usize {
    m.parts
        .iter()
        .map(|p| match p {
            Part::Text { text } | Part::Thinking { text, .. } => text.len(),
            Part::ToolCall { args, .. } => args.to_string().len(),
            Part::ToolResult { content, .. } => content.len(),
            Part::Image { .. } => IMAGE_CHARS,
        })
        .sum()
}

/// The tokens a request takes, estimated: its system prompt, its tools' specs and its messages.
pub fn estimate_tokens(req: &ChatRequest) -> u64 {
    let system: usize = req.system.iter().map(|b| b.text.len()).sum();
    let tools: usize = req
        .tools
        .iter()
        .map(|t| t.name.len() + t.description.len() + t.parameters.to_string().len())
        .sum();
    let messages: usize = req.messages.iter().map(message_chars).sum();
    (system + tools + messages) as u64 / CHARS_PER_TOKEN
}

fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_owned()
    } else {
        format!("{}…[cut]", s.chars().take(n).collect::<String>())
    }
}

/// A message as transcript text: who said it, its text, its tool calls and results (cut), no thinking.
pub fn transcript(m: &Message) -> String {
    let who = match m.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    let mut out = String::new();
    for p in &m.parts {
        match p {
            Part::Text { text } if !text.trim().is_empty() => {
                out.push_str(&format!("[{who}] {}\n", text.trim()))
            }
            Part::ToolCall { name, args, .. } => out.push_str(&format!(
                "[{who} calls {name}] {}\n",
                cut(&args.to_string(), PIECE_PART_CHARS)
            )),
            Part::ToolResult {
                name,
                content,
                is_error,
                ..
            } => out.push_str(&format!(
                "[{name} {}] {}\n",
                if *is_error { "failed" } else { "returned" },
                cut(content, PIECE_PART_CHARS)
            )),
            Part::Image { .. } => out.push_str(&format!("[{who}] [an image]\n")),
            _ => {}
        }
    }
    out
}

/// The transcript of `messages` in pieces of at most `budget` characters (a longer message is cut to fit).
pub fn pieces(messages: &[Message], budget: usize) -> Vec<String> {
    let budget = budget.max(2_000);
    let mut out = Vec::new();
    let mut piece = String::new();
    for m in messages {
        let mut t = transcript(m);
        if t.len() > budget {
            t = cut(&t, budget * 9 / 10);
        }
        if !piece.is_empty() && piece.len() + t.len() > budget {
            out.push(std::mem::take(&mut piece));
        }
        piece.push_str(&t);
    }
    if !piece.trim().is_empty() {
        out.push(piece);
    }
    out
}

impl Run<'_> {
    /// The tokens the run's next request would take, estimated.
    pub(super) fn estimate(&self, order: &WorkOrder, messages: &[Message]) -> u64 {
        estimate_tokens(&self.request(order, messages, true))
    }

    /// Folds `messages[head..]`, all but the recent tail, into notes written piece by piece, and puts the notes in
    /// place of that part. `Ok(false)` when there is nothing older to fold or the model wrote no notes.
    pub(super) async fn fit(
        &self,
        order: &WorkOrder,
        messages: &mut Vec<Message>,
        head: usize,
        usage: &mut Usage,
    ) -> Result<bool, Outcome> {
        let window = self.cfg.context_window;
        let split = tail_start(
            messages,
            head,
            (window * RETAIN_PERCENT / 100 * CHARS_PER_TOKEN) as usize,
        );
        if split <= head {
            return Ok(false);
        }
        // What one notes request carries besides the piece: the system prompt and the tools (listed, not callable),
        // the notes so far and room for the new notes.
        let fixed = self.estimate(order, &messages[..head.min(messages.len())])
            + NOTES_ROOM
            + self.cfg.max_tokens.min(8_000) as u64;
        let room = window.saturating_sub(fixed) * PIECE_PERCENT / 100;
        let parts = pieces(&messages[head..split], (room * CHARS_PER_TOKEN) as usize);
        let total = parts.len();
        let mut notes = String::new();
        for (k, piece) in parts.iter().enumerate() {
            let ask = format!(
                "{FOLD}\n\n<notes>\n{}\n</notes>\n\n<conversation piece=\"{} of {total}\">\n{piece}</conversation>",
                notes.trim(),
                k + 1
            );
            let mut msgs = messages[..head].to_vec();
            msgs.push(Message::user_text(ask));
            let r = self.turn(&self.notes_request(order, &msgs)).await?;
            usage.add(&r.usage);
            self.observer.response(&r.usage);
            let text = r.message.text();
            if text.trim().is_empty() || r.stop == StopReason::MaxTokens {
                return Ok(false);
            }
            notes = text;
        }
        let (read, changed) = files_touched(&messages[head..split]);
        let mut tail = messages.split_off(split);
        strip_thinking(&mut tail);
        messages.truncate(head);
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
            "fit",
            &format!(
                "{} earlier messages folded into {} characters of notes in {total} piece(s), to fit a {window}-token window",
                split - head,
                notes.trim().len()
            ),
        );
        messages.push(Message::user_text(text));
        messages.extend(tail);
        tracing::info!(
            pieces = total,
            "conversation folded to fit the context window"
        );
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_conversation_is_cut_into_pieces_that_fit() {
        let msgs: Vec<Message> = (0..40)
            .map(|i| Message::user_text(format!("step {i} {}", "x".repeat(900))))
            .collect();
        let p = pieces(&msgs, 5_000);
        assert!(p.len() >= 7, "{}", p.len());
        assert!(p.iter().all(|s| s.len() <= 5_000 + 1_000));
        assert!(p[0].starts_with("[user] step 0"));
        // A message longer than a piece is cut, not dropped.
        let one = pieces(&[Message::user_text("y".repeat(50_000))], 5_000);
        assert_eq!(one.len(), 1);
        assert!(one[0].ends_with("…[cut]\n") || one[0].contains("…[cut]"));
    }
}
