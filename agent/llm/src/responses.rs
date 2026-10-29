//! OpenAI Responses API adapter (`POST /responses`, streamed). Used by the OpenCode relay for the model families it
//! serves only over this wire (`gpt-*`, `grok-*`, `muse-spark*`). Stateless: `store: false`, the whole conversation
//! is sent each turn; reasoning is not replayed (it would need the encrypted reasoning items).
//!
//! Wire: instructions = the system blocks; input items = `{role, content: [input_text | input_image | output_text]}`,
//! `{type: function_call, call_id, name, arguments}`, `{type: function_call_output, call_id, output}`; stream events
//! `response.output_text.delta`, `response.reasoning_summary_text.delta`, `response.output_item.added/done`,
//! `response.function_call_arguments.delta`, `response.refusal.delta`, `response.completed`,
//! `response.incomplete`, `response.failed`, `error`.

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::LlmError;
use crate::http::Http;
use crate::openai::{map_error, session_key};
use crate::sse::SseEvent;
use crate::types::*;
use crate::Provider;

pub const KIND: &str = "openai_responses";

pub struct Responses {
    id: String,
    base_url: String,
    api_key: Option<String>,
    http: Http,
    /// Send the OpenCode relay's `x-opencode-session` affinity header.
    opencode: bool,
}

impl Responses {
    pub fn new(
        id: &str,
        base_url: &str,
        api_key: Option<&str>,
        http: Http,
        opencode: bool,
    ) -> Self {
        Self {
            id: id.to_owned(),
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key: api_key.map(str::to_owned),
            http,
            opencode,
        }
    }

    fn headers(&self, req: &ChatRequest) -> Vec<(&'static str, String)> {
        let mut h = Vec::new();
        if let Some(k) = &self.api_key {
            h.push(("authorization", format!("Bearer {k}")));
        }
        if self.opencode {
            h.push(("x-opencode-session", session_key(req)));
        }
        h
    }
}

pub fn build_body(req: &ChatRequest) -> Value {
    let mut input: Vec<Value> = Vec::new();
    for m in &req.messages {
        match m.role {
            Role::User => {
                for p in &m.parts {
                    if let Part::ToolResult {
                        call_id,
                        content,
                        is_error,
                        ..
                    } = p
                    {
                        let output = if *is_error {
                            format!("ERROR: {content}")
                        } else {
                            content.clone()
                        };
                        input.push(json!({"type": "function_call_output", "call_id": call_id, "output": output}));
                    }
                }
                let content: Vec<Value> = m
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::Text { text } if !text.is_empty() => {
                            Some(json!({"type": "input_text", "text": text}))
                        }
                        Part::Image {
                            media_type,
                            data_b64,
                        } => Some(json!({"type": "input_image",
                            "image_url": format!("data:{media_type};base64,{data_b64}")})),
                        _ => None,
                    })
                    .collect();
                if !content.is_empty() {
                    input.push(json!({"role": "user", "content": content}));
                }
            }
            Role::Assistant => {
                let text = m.text();
                if !text.is_empty() {
                    input.push(json!({"role": "assistant", "content": [{"type": "output_text", "text": text}]}));
                }
                for p in &m.parts {
                    if let Part::ToolCall { id, name, args, .. } = p {
                        input.push(json!({"type": "function_call", "call_id": id, "name": name,
                            "arguments": args.to_string()}));
                    }
                }
            }
        }
    }
    let instructions: Vec<&str> = req
        .system
        .iter()
        .map(|b| b.text.as_str())
        .filter(|t| !t.is_empty())
        .collect();
    let mut body = json!({
        "model": req.model,
        "stream": true,
        "store": false,
        "input": input,
        "max_output_tokens": req.max_tokens,
    });
    if !instructions.is_empty() {
        body["instructions"] = json!(instructions.join("\n\n"));
    }
    if let Some(k) = &req.cache_key {
        body["prompt_cache_key"] = json!(k);
    }
    if !req.tools.is_empty() && req.tools_off {
        body["tool_choice"] = json!("none");
    }
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(
            req.tools
                .iter()
                .map(|t| {
                    json!({"type": "function", "name": t.name, "description": t.description,
                    "parameters": t.parameters})
                })
                .collect(),
        );
    }
    // The model's own level (ADR-114); none for a model whose levels are unknown.
    if let Some(e) = crate::effort::level_for(&req.model, &req.effort) {
        body["reasoning"] = json!({"effort": e.as_str()});
    }
    body
}

#[derive(Default)]
pub struct StreamState {
    text: String,
    reasoning: String,
    refusal: String,
    /// output_index -> (call_id, name, arguments)
    calls: BTreeMap<u64, (String, String, String)>,
    finish: Option<StopReason>,
    usage: Usage,
    model: String,
}

fn stream_error(e: &Value) -> LlmError {
    let kind = e["code"]
        .as_str()
        .or(e["type"].as_str())
        .unwrap_or("error")
        .to_owned();
    LlmError::Stream {
        retryable: kind.contains("overload") || kind.contains("server") || kind.contains("rate"),
        message: e["message"].as_str().unwrap_or_default().to_owned(),
        kind,
    }
}

impl StreamState {
    pub fn apply(
        &mut self,
        ev: &SseEvent,
        emit: &(dyn Fn(ChatEvent) + Send + Sync),
    ) -> Result<bool, LlmError> {
        let data = ev.data.trim();
        if data.is_empty() || data == "[DONE]" {
            return Ok(false);
        }
        let v: Value =
            serde_json::from_str(data).map_err(|e| LlmError::Decode(format!("{e}: {data}")))?;
        let kind = v["type"].as_str().or(ev.event.as_deref()).unwrap_or("");
        match kind {
            "response.output_text.delta" => {
                if let Some(d) = v["delta"].as_str() {
                    self.text.push_str(d);
                    emit(ChatEvent::TextDelta(d.to_owned()));
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                if let Some(d) = v["delta"].as_str() {
                    self.reasoning.push_str(d);
                    emit(ChatEvent::ThinkingDelta(d.to_owned()));
                }
            }
            "response.refusal.delta" => {
                if let Some(d) = v["delta"].as_str() {
                    self.refusal.push_str(d);
                }
            }
            "response.output_item.added" | "response.output_item.done" => {
                let item = &v["item"];
                if item["type"] == "function_call" {
                    let i = v["output_index"].as_u64().unwrap_or(0);
                    let added = kind.ends_with("added");
                    let e = self.calls.entry(i).or_default();
                    if let Some(c) = item["call_id"].as_str() {
                        e.0 = c.to_owned();
                    }
                    if let Some(n) = item["name"].as_str() {
                        if e.1.is_empty() {
                            e.1 = n.to_owned();
                            if added {
                                emit(ChatEvent::ToolCallStarted {
                                    id: e.0.clone(),
                                    name: e.1.clone(),
                                });
                            }
                        }
                    }
                    if !added {
                        if let Some(a) = item["arguments"].as_str() {
                            e.2 = a.to_owned();
                        }
                    }
                }
            }
            "response.function_call_arguments.delta" => {
                let i = v["output_index"].as_u64().unwrap_or(0);
                if let Some(d) = v["delta"].as_str() {
                    self.calls.entry(i).or_default().2.push_str(d);
                }
            }
            "response.completed" | "response.incomplete" => {
                let r = &v["response"];
                if let Some(m) = r["model"].as_str() {
                    self.model = m.to_owned();
                }
                let u = &r["usage"];
                let input = u["input_tokens"].as_u64().unwrap_or(0);
                let cached = u["input_tokens_details"]["cached_tokens"]
                    .as_u64()
                    .unwrap_or(0);
                self.usage = Usage {
                    input: input.saturating_sub(cached),
                    output: u["output_tokens"].as_u64().unwrap_or(0),
                    cache_read: cached,
                    cache_write: 0,
                    cache_write_1h: 0,
                    reasoning: u["output_tokens_details"]["reasoning_tokens"]
                        .as_u64()
                        .unwrap_or(0),
                };
                self.finish = Some(if kind == "response.incomplete" {
                    match r["incomplete_details"]["reason"].as_str() {
                        Some("content_filter") => StopReason::Refusal,
                        _ => StopReason::MaxTokens,
                    }
                } else {
                    StopReason::EndTurn
                });
                return Ok(true);
            }
            "response.failed" => return Err(stream_error(&v["response"]["error"])),
            "error" => return Err(stream_error(&v)),
            _ => {}
        }
        Ok(false)
    }

    pub fn finish(mut self) -> Result<ChatResponse, LlmError> {
        let Some(finish) = self.finish.take() else {
            return Err(LlmError::StreamDropped(
                "stream ended before response.completed".into(),
            ));
        };
        let mut parts = Vec::new();
        if !self.reasoning.is_empty() {
            parts.push(Part::Thinking {
                text: std::mem::take(&mut self.reasoning),
                signature: None,
                provider: KIND.into(),
                redacted: false,
                model: None,
            });
        }
        if !self.text.is_empty() {
            parts.push(Part::Text {
                text: std::mem::take(&mut self.text),
            });
        }
        for (id, name, args) in std::mem::take(&mut self.calls).into_values() {
            parts.push(Part::ToolCall {
                // A unique id, not the call's index in this response (`history::fallback_call_id`).
                id: if id.is_empty() {
                    crate::history::fallback_call_id("call")
                } else {
                    id
                },
                name,
                args: crate::anthropic::parse_args(&args),
                signature: None,
            });
        }
        let has_calls = parts.iter().any(|p| matches!(p, Part::ToolCall { .. }));
        let stop = if !self.refusal.is_empty() && !has_calls {
            parts.push(Part::Text {
                text: std::mem::take(&mut self.refusal),
            });
            StopReason::Refusal
        } else if has_calls && finish == StopReason::EndTurn {
            StopReason::ToolUse
        } else {
            finish
        };
        Ok(ChatResponse {
            message: Message {
                role: Role::Assistant,
                parts,
            },
            stop,
            usage: self.usage,
            model: self.model,
            dropped_thinking: 0,
            refusal: None,
        })
    }
}

#[async_trait]
impl Provider for Responses {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &'static str {
        KIND
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError> {
        let h: Vec<(&'static str, String)> = self
            .api_key
            .as_ref()
            .map(|k| vec![("authorization", format!("Bearer {k}"))])
            .unwrap_or_default();
        let v = self
            .http
            .get_json(&format!("{}/models", self.base_url), &h, map_error)
            .await?;
        Ok(v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| ModelInfo {
                id: m["id"].as_str().unwrap_or_default().to_owned(),
                display_name: None,
                context_window: None,
                max_output: None,
                supports_tools: Some(true),
                supports_vision: None,
                efforts: None,
                default_effort: None,
            })
            .map(|mut i| {
                crate::effort::fill(&mut i);
                i
            })
            .collect())
    }

    async fn chat(
        &self,
        req: &ChatRequest,
        emit: &(dyn Fn(ChatEvent) + Send + Sync),
    ) -> Result<ChatResponse, LlmError> {
        let body = build_body(req);
        let mut st = StreamState::default();
        let r = self
            .http
            .post_sse(
                &format!("{}/responses", self.base_url),
                &self.headers(req),
                &body,
                map_error,
                |ev| st.apply(&ev, emit),
            )
            .await;
        match r {
            Ok(()) => {}
            Err(LlmError::StreamDropped(_)) if st.finish.is_some() => {}
            Err(e) => return Err(e),
        }
        let resp = st.finish()?;
        for c in resp.message.tool_calls() {
            emit(ChatEvent::ToolCall(c));
        }
        emit(ChatEvent::Usage(resp.usage));
        emit(ChatEvent::Done);
        Ok(resp)
    }
}
