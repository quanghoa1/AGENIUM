//! The raw traces of a closed ticket that later tickets of its project may be reminded of (ADR-158 decision 31): what
//! the review said, the questions and their answers, a handoff, a failed merge, and the report the review approved,
//! cut into passages. Measured on four projects (decision 30): earlier reports of a reopened ticket describe work the
//! review refused and were the most misleading notes, progress lines are passing, so neither is kept.

use agenium_core::{Comment, CommentBody};
use serde_json::Value;

/// About how long a passage is: a paragraph or two, as measured.
pub const CHUNK: usize = 700;

/// One passage of a ticket's traces: its comment's kind, when it was written, its text.
#[derive(Debug, Clone, PartialEq)]
pub struct Trace {
    pub kind: &'static str,
    pub at: String,
    pub text: String,
}

/// Field names that carry no words of their own.
const SKIP: &[&str] = &[
    "ticket_id",
    "ac_id",
    "type",
    "status",
    "decision",
    "category",
    "eta",
    "percent",
    "kind",
    "ref",
];

fn words(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) if !s.trim().is_empty() => out.push(s.trim().to_owned()),
        Value::Array(a) => a.iter().for_each(|x| words(x, out)),
        Value::Object(o) => o
            .iter()
            .filter(|(k, _)| !SKIP.contains(&k.as_str()))
            .for_each(|(_, x)| words(x, out)),
        _ => {}
    }
}

/// `text` in passages of about `size` characters, cut at lines, then at spaces; tiny ones dropped.
pub fn chunks(text: &str, size: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        let mut line = line.trim_end();
        while line.chars().count() > size {
            let end = line
                .char_indices()
                .nth(size)
                .map(|(i, _)| i)
                .unwrap_or(line.len());
            let cut = line[..end]
                .rfind(' ')
                .filter(|c| *c > end / 2)
                .unwrap_or(end);
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(line[..cut].to_owned());
            line = line[cut..].trim_start();
        }
        if !cur.is_empty() && cur.chars().count() + line.chars().count() + 1 > size {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push('\n');
        }
        cur.push_str(line);
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out.into_iter()
        .map(|c| c.trim().to_owned())
        .filter(|c| c.chars().count() >= 40)
        .collect()
}

/// The traces kept of a closed ticket's comments (in their order).
pub fn of_ticket(comments: &[Comment]) -> Vec<Trace> {
    use CommentBody as B;
    let last_report = comments
        .iter()
        .rposition(|c| matches!(c.body, B::Report(_)));
    let mut out = Vec::new();
    for (i, c) in comments.iter().enumerate() {
        let kind = match &c.body {
            B::Report(_) if Some(i) == last_report => "report",
            B::Review(_) => "review",
            B::Question(_) => "question",
            B::Answer(_) => "answer",
            B::Handoff(_) => "handoff",
            B::System { text } if text.starts_with("Merge failed") => "merge",
            _ => continue,
        };
        let v = serde_json::to_value(&c.body).unwrap_or(Value::Null);
        let mut w = Vec::new();
        words(v.get("data").unwrap_or(&v), &mut w);
        for text in chunks(&w.join("\n"), CHUNK) {
            out.push(Trace {
                kind,
                at: c.created_at.clone(),
                text,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use agenium_core::{Author, CommentId, TicketId};
    use serde_json::json;

    fn comment(body: Value) -> Comment {
        let body: CommentBody = serde_json::from_value(body).unwrap();
        Comment {
            id: CommentId::new(),
            ticket: TicketId::new(),
            ticket_key: "AGN-1".parse().unwrap(),
            kind: body.kind(),
            author: Author::System,
            reply_to: None,
            body,
            blocking: false,
            resolved: false,
            created_at: "2026-11-06T08:00:00.000Z".into(),
        }
    }

    fn report(summary: &str) -> Value {
        json!({"type": "Report", "data": {"ticket_id": "AGN-1", "summary": summary, "confidence": 0.9,
            "acceptance": [{"ac_id": "AC-1", "status": "Met", "evidence": "node --test passes 12 tests"}]}})
    }

    #[test]
    fn the_approved_report_reviews_answers_and_failed_merges_are_kept() {
        let all = vec![
            comment(report(
                "First attempt: the store has nine functions and writes in place.",
            )),
            comment(
                json!({"type": "Review", "data": {"ticket_id": "AGN-1", "decision": "Reopen",
                "ac_feedback": [{"ac_id": "AC-1", "feedback": "The store must write through a temp file and rename."}]}}),
            ),
            comment(
                json!({"type": "System", "data": {"text": "Merge failed: the pre-merge check failed: connect EACCES 127.0.0.1"}}),
            ),
            comment(
                json!({"type": "System", "data": {"text": "Merged into main at abc123 (2 files). The ticket stays Resolved."}}),
            ),
            comment(report(
                "Second attempt: the store writes a temp file and renames it; thirteen functions.",
            )),
        ];
        let t = of_ticket(&all);
        let kinds: Vec<&str> = t.iter().map(|x| x.kind).collect();
        assert_eq!(kinds, ["review", "merge", "report"], "{t:?}");
        let text: String = t.iter().map(|x| x.text.as_str()).collect();
        assert!(
            text.contains("temp file and rename")
                && text.contains("EACCES")
                && text.contains("Second attempt")
        );
        assert!(
            !text.contains("First attempt"),
            "the refused report is left out"
        );
        assert!(
            !text.contains("Merged into main"),
            "a passing notice is left out"
        );
        assert!(
            !text.contains("Reopen") && !text.contains("AC-1"),
            "field values that are not words are left out"
        );
        assert_eq!(t[0].at, "2026-11-06T08:00:00.000Z");
    }

    #[test]
    fn long_text_is_cut_into_passages_and_crumbs_dropped() {
        let para = "word ".repeat(400);
        let c = chunks(
            &format!("short\n{para}\nlast line that is long enough to keep as one"),
            700,
        );
        assert!(c.len() >= 3, "{}", c.len());
        assert!(c.iter().all(|p| p.chars().count() <= 700));
        assert!(chunks("tiny", 700).is_empty());
    }
}
