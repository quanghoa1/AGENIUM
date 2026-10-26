//! The conversation as a provider will accept it (ADR-101). Stored history keeps everything; this pass runs on the
//! copy sent with each request, so one bad block can never poison every later request (the most damaging class of
//! bug in other agents: a dangling tool call, an empty block or a foreign signature makes every request fail):
//! - thinking written by another model is dropped (a signature is valid for its own model only);
//! - a tool call without a result gets one ("not run"), a result without its call is dropped;
//! - empty text blocks and empty messages are dropped; an empty tool result says so;
//! - a tool-call id is made safe for every provider (`[A-Za-z0-9_-]`, at most 64 characters), the same way on the
//!   call and on its result.

use std::collections::HashSet;

use crate::{Message, Part, Role};

/// Content of the result added for a tool call that never got one.
pub const NOT_RUN: &str = "not run: the turn ended before this call was executed";
/// Content sent for a tool result that is empty.
pub const NO_OUTPUT: &str = "(no output)";

/// A tool-call id every provider accepts; ids that are already safe stay as they are.
pub fn wire_id(id: &str) -> String {
    let clean: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if clean.is_empty() {
        return "call".into();
    }
    if clean == id && clean.len() <= 64 {
        return clean;
    }
    // Changed or too long: keep it unique with a hash of the original.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    let suffix = format!("_{h:016x}");
    let keep = 64 - suffix.len();
    format!("{}{suffix}", &clean[..clean.len().min(keep)])
}

/// The history to send to `model`.
pub fn prepare(messages: &[Message], model: &str) -> Vec<Message> {
    let results: HashSet<&str> = messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .filter_map(|p| match p {
            Part::ToolResult { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        })
        .collect();
    let mut calls_seen: HashSet<String> = HashSet::new();
    let mut out: Vec<Message> = Vec::with_capacity(messages.len());
    for m in messages {
        let mut parts = Vec::with_capacity(m.parts.len());
        let mut missing = Vec::new();
        for p in &m.parts {
            match p {
                Part::Thinking {
                    model: Some(by), ..
                } if by != model => {}
                Part::Text { text } if text.trim().is_empty() => {}
                Part::ToolCall {
                    id,
                    name,
                    args,
                    signature,
                } => {
                    calls_seen.insert(id.clone());
                    if !results.contains(id.as_str()) {
                        missing.push((wire_id(id), name.clone()));
                    }
                    parts.push(Part::ToolCall {
                        id: wire_id(id),
                        name: name.clone(),
                        args: args.clone(),
                        signature: signature.clone(),
                    });
                }
                Part::ToolResult {
                    call_id,
                    name,
                    content,
                    is_error,
                } => {
                    if !calls_seen.contains(call_id) {
                        continue;
                    }
                    parts.push(Part::ToolResult {
                        call_id: wire_id(call_id),
                        name: name.clone(),
                        content: if content.trim().is_empty() {
                            NO_OUTPUT.into()
                        } else {
                            content.clone()
                        },
                        is_error: *is_error,
                    });
                }
                other => parts.push(other.clone()),
            }
        }
        if !parts.is_empty() {
            out.push(Message {
                role: m.role,
                parts,
            });
        }
        if !missing.is_empty() {
            out.push(Message {
                role: Role::User,
                parts: missing
                    .into_iter()
                    .map(|(call_id, name)| Part::ToolResult {
                        call_id,
                        name,
                        content: NOT_RUN.into(),
                        is_error: true,
                    })
                    .collect(),
            });
        }
    }
    // The results added for unanswered calls sit right after their call; merge them with the next user message
    // when that holds results too, so the results of one turn arrive together.
    let mut merged: Vec<Message> = Vec::with_capacity(out.len());
    for m in out {
        let both_results = |a: &Message, b: &Message| {
            a.role == Role::User
                && b.role == Role::User
                && a.parts.iter().all(|p| matches!(p, Part::ToolResult { .. }))
                && b.parts
                    .first()
                    .is_some_and(|p| matches!(p, Part::ToolResult { .. }))
        };
        match merged.last_mut() {
            Some(last) if both_results(last, &m) => last.parts.extend(m.parts),
            _ => merged.push(m),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(id: &str) -> Part {
        Part::ToolCall {
            id: id.into(),
            name: "read_file".into(),
            args: json!({}),
            signature: None,
        }
    }

    fn result(id: &str, content: &str) -> Part {
        Part::ToolResult {
            call_id: id.into(),
            name: "read_file".into(),
            content: content.into(),
            is_error: false,
        }
    }

    fn msg(role: Role, parts: Vec<Part>) -> Message {
        Message { role, parts }
    }

    #[test]
    fn ids_every_provider_accepts() {
        assert_eq!(wire_id("toolu_01ABC-x"), "toolu_01ABC-x");
        let a = wire_id("fc_123|resp_456");
        assert!(a.starts_with("fc_123_resp_456_") && a.len() <= 64, "{a}");
        assert_ne!(
            wire_id("a|b"),
            wire_id("a:b"),
            "different ids stay different"
        );
        let long = wire_id(&"x".repeat(100));
        assert_eq!(long.len(), 64);
        assert_eq!(wire_id(""), "call");
    }

    #[test]
    fn unanswered_calls_get_a_result_and_stray_results_go() {
        let h = vec![
            msg(
                Role::User,
                vec![Part::Text {
                    text: "do it".into(),
                }],
            ),
            msg(Role::Assistant, vec![call("c1"), call("c2")]),
            msg(Role::User, vec![result("c1", "ok"), result("zz", "orphan")]),
        ];
        let p = prepare(&h, "m");
        assert_eq!(p.len(), 3, "{p:#?}");
        let ids: Vec<(&str, &str)> = p[2]
            .parts
            .iter()
            .filter_map(|x| match x {
                Part::ToolResult {
                    call_id, content, ..
                } => Some((call_id.as_str(), content.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(ids, [("c2", NOT_RUN), ("c1", "ok")]);
    }

    #[test]
    fn a_dangling_call_at_the_end_is_closed() {
        let h = vec![
            msg(Role::User, vec![Part::Text { text: "go".into() }]),
            msg(Role::Assistant, vec![call("c1")]),
            msg(
                Role::User,
                vec![Part::Text {
                    text: "resume".into(),
                }],
            ),
        ];
        let p = prepare(&h, "m");
        assert_eq!(p.len(), 4);
        assert!(matches!(&p[2].parts[0], Part::ToolResult { content, .. } if content == NOT_RUN));
        assert_eq!(p[3].text(), "resume");
    }

    #[test]
    fn empty_blocks_and_foreign_thinking() {
        let thinking = |model: Option<&str>| Part::Thinking {
            text: "hmm".into(),
            signature: Some("sig".into()),
            provider: "anthropic".into(),
            redacted: false,
            model: model.map(str::to_owned),
        };
        let h = vec![
            msg(Role::User, vec![Part::Text { text: " \n".into() }]),
            msg(
                Role::Assistant,
                vec![
                    thinking(Some("claude-a")),
                    thinking(Some("claude-b")),
                    thinking(None),
                    call("c1"),
                ],
            ),
            msg(Role::User, vec![result("c1", "\r\n")]),
        ];
        let p = prepare(&h, "claude-b");
        assert_eq!(p.len(), 2, "the empty user message is gone");
        let models: Vec<Option<String>> = p[0]
            .parts
            .iter()
            .filter_map(|x| match x {
                Part::Thinking { model, .. } => Some(model.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(models, [Some("claude-b".into()), None]);
        assert!(matches!(&p[1].parts[0], Part::ToolResult { content, .. } if content == NO_OUTPUT));
    }
}
