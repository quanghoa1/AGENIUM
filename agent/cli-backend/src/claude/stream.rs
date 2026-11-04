//! Claude Code's stream (`stream-json`): one line's events, as the main agent's; `tool_result` content as text.

use agenium_llm::Usage;
use serde_json::Value;

use super::Claude;
use crate::driver::Event;

/// A `tool_result`'s content as text: a string, or its blocks (text; a tool search's `tool_reference` by its name).
pub(super) fn result_text(c: &Value) -> String {
    match c {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| match b["type"].as_str() {
                Some("text") => b["text"].as_str().map(str::to_owned),
                Some("tool_reference") => b["tool_name"].as_str().map(|n| format!("tool: {n}")),
                Some(other) => Some(format!("[{other}]")),
                None => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

impl Claude {
    /// The events of one stream line, as the main agent's (a subagent's are wrapped by `parse`).
    pub(super) fn parse_own(&self, v: &Value) -> Vec<Event> {
        match v["type"].as_str() {
            Some("system") if v["subtype"] == "init" => v["session_id"]
                .as_str()
                .map(|s| vec![Event::Session(s.to_owned())])
                .unwrap_or_default(),
            Some("assistant") => {
                let mut out: Vec<Event> = v["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|c| match c["type"].as_str() {
                        Some("text") => c["text"]
                            .as_str()
                            .filter(|t| !t.is_empty())
                            .map(|t| Event::Text(format!("{t}\n"))),
                        Some("tool_use") => Some(Event::Tool {
                            id: c["id"].as_str().unwrap_or("").to_owned(),
                            name: c["name"].as_str().unwrap_or("tool").to_owned(),
                            input: c["input"].clone(),
                            done: false,
                        }),
                        _ => None,
                    })
                    .collect();
                // The request's own usage: what the model read, its context (ADR-155).
                let u = &v["message"]["usage"];
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                let context = n("input_tokens")
                    + n("cache_read_input_tokens")
                    + n("cache_creation_input_tokens")
                    + n("output_tokens");
                if context > 0 {
                    out.push(Event::Context(context));
                }
                out
            }
            Some("user") => v["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|c| c["type"] == "tool_result")
                .flat_map(|c| {
                    let id = c["tool_use_id"].as_str().unwrap_or("").to_owned();
                    [
                        Event::ToolOutput {
                            id: id.clone(),
                            text: result_text(&c["content"]),
                            error: c["is_error"] == true,
                        },
                        Event::Tool {
                            id,
                            name: String::new(),
                            input: Value::Null,
                            done: true,
                        },
                    ]
                })
                .collect(),
            Some("rate_limit_event") => {
                agenium_llm::vendor_usage::from_claude_event(v, &agenium_core::now_iso())
                    .map(Event::Limits)
                    .into_iter()
                    .collect()
            }
            Some("result") => {
                let u = &v["usage"];
                let n = |k: &str| u[k].as_u64().unwrap_or(0);
                vec![Event::TurnEnd {
                    usage: Usage {
                        input: n("input_tokens"),
                        output: n("output_tokens"),
                        cache_read: n("cache_read_input_tokens"),
                        cache_write: n("cache_creation_input_tokens"),
                        cache_write_1h: 0,
                        reasoning: u["output_tokens_details"]["thinking_tokens"]
                            .as_u64()
                            .unwrap_or(0),
                    },
                    response: v["result"].as_str().unwrap_or("").to_owned(),
                    denied: vec![],
                    error: (v["is_error"] == true).then(|| {
                        format!(
                            "claude turn ended with {}: {}",
                            v["subtype"].as_str().unwrap_or("an error"),
                            v["result"].as_str().unwrap_or("")
                        )
                    }),
                }]
            }
            _ => vec![],
        }
    }
}
