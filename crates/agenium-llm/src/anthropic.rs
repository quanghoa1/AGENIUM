//! Anthropic Messages API adapter (raw HTTP + SSE; there is no official Rust SDK).
//!
//! API version `2026-10-21`: no sampling parameters; reasoning depth via `output_config.effort`; adaptive thinking
//! on 4.6+ models (`budget_tokens` only on older models); `tool_choice` stays `auto` (forced choice is rejected by
//! current models). Thinking blocks are replayed unchanged with their signature (append-only history keeps them
//! valid). Prompt caching: breakpoints after the tools, after the stable system blocks and on the last message
//! (decision 13), 1 hour TTL for long-running agents (the longer TTL must come first).

use std::collections::BTreeMap;

use agenium_core::Effort;
use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde_json::{json, Value};

use crate::error::{parse_retry_after, LlmError};
use crate::http::{header, Http};
use crate::sse::SseEvent;
use crate::types::*;
use crate::Provider;

pub const API_VERSION: &str = "2026-10-21";
pub const KIND: &str = "anthropic";

/// How a model takes reasoning settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingMode {
    /// `thinking: {type: "adaptive"}` + `output_config.effort` (Claude 4.6 and later).
    Adaptive { xhigh: bool },
    /// `thinking: {type: "enabled", budget_tokens}`; effort only where `effort` is true (Opus 4.5).
    Budget { effort: bool },
}

pub fn thinking_mode(model: &str) -> ThinkingMode {
    let m = model.to_ascii_lowercase();
    let old = [
        "claude-3",
        "claude-haiku-4-5",
        "claude-sonnet-4-5",
        "claude-opus-4-1",
        "claude-opus-4-0",
        "claude-sonnet-4-0",
        "claude-opus-4-2",
        "claude-sonnet-4-2",
    ];
    if m.starts_with("claude-opus-4-5") {
        return ThinkingMode::Budget { effort: true };
    }
    if old.iter().any(|p| m.starts_with(p)) {
        return ThinkingMode::Budget { effort: false };
    }
    ThinkingMode::Adaptive {
        xhigh: !(m.starts_with("claude-opus-4-6") || m.starts_with("claude-sonnet-4-6")),
    }
}

fn budget(effort: Effort, max_tokens: u32) -> u32 {
    let b = match effort {
        Effort::Low => 1024,
        Effort::Medium => 8192,
        Effort::High => 24_576,
        Effort::Max => 32_000,
    };
    // budget_tokens must be below max_tokens.
    b.min(max_tokens.saturating_sub(1024)).max(1024)
}

pub struct Anthropic {
    id: String,
    base_url: String,
    api_key: String,
    http: Http,
}

impl Anthropic {
    pub fn new(id: &str, base_url: Option<&str>, api_key: &str, http: Http) -> Self {
        Self {
            id: id.to_owned(),
            base_url: base_url
                .unwrap_or("https://api.anthropic.com")
                .trim_end_matches('/')
                .to_owned(),
            api_key: api_key.to_owned(),
            http,
        }
    }

    fn headers(&self) -> Vec<(&'static str, String)> {
        vec![
            ("x-api-key", self.api_key.clone()),
            ("anthropic-version", API_VERSION.to_owned()),
        ]
    }
}

pub fn map_error(status: u16, h: &HeaderMap, body: &str) -> LlmError {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let kind = v["error"]["type"].as_str().unwrap_or("").to_owned();
    let message = v["error"]["message"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| body.chars().take(500).collect());
    // Credit exhaustion arrives as 402 billing_error, or historically as 400 "credit balance is too low".
    let quota = status == 402
        || kind == "billing_error"
        || message.to_ascii_lowercase().contains("credit balance");
    LlmError::Http {
        status,
        kind,
        message,
        quota,
        retry_after: parse_retry_after(header(h, "retry-after").as_deref()),
    }
}

fn cache_control(ttl_1h: bool) -> Value {
    if ttl_1h {
        json!({"type": "ephemeral", "ttl": "1h"})
    } else {
        json!({"type": "ephemeral"})
    }
}

/// Builds the request body (pure; covered by contract tests).
pub fn build_body(req: &ChatRequest) -> Value {
    let long = req.long_cache;
    let mut tools: Vec<Value> = req
        .tools
        .iter()
        .map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.parameters, "eager_input_streaming": true}))
        .collect();
    if let Some(last) = tools.last_mut() {
        last["cache_control"] = cache_control(long);
    }
    // At most 2 system breakpoints so tools + system + last message stay within the 4 allowed.
    let mut sys_breaks = 0;
    let system: Vec<Value> = req
        .system
        .iter()
        .filter(|b| !b.text.is_empty())
        .map(|b| {
            let mut v = json!({"type": "text", "text": b.text});
            if b.cache && sys_breaks < 2 {
                sys_breaks += 1;
                v["cache_control"] = cache_control(long);
            }
            v
        })
        .collect();
    let mut messages: Vec<Value> = Vec::new();
    for m in &req.messages {
        let role = match m.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        let mut blocks: Vec<Value> = Vec::new();
        for p in &m.parts {
            match p {
                Part::Text { text } if !text.is_empty() => blocks.push(json!({"type": "text", "text": text})),
                Part::Text { .. } => {}
                Part::Thinking { text, signature, provider, redacted } if provider == KIND && m.role == Role::Assistant => {
                    if *redacted {
                        blocks.push(json!({"type": "redacted_thinking", "data": signature.clone().unwrap_or_default()}));
                    } else {
                        blocks.push(json!({"type": "thinking", "thinking": text, "signature": signature.clone().unwrap_or_default()}));
                    }
                }
                Part::Thinking { .. } => {}
                Part::ToolCall { id, name, args, .. } => blocks.push(json!({"type": "tool_use", "id": id, "name": name, "input": args})),
                Part::ToolResult { call_id, content, is_error, .. } => {
                    blocks.push(json!({"type": "tool_result", "tool_use_id": call_id, "content": content, "is_error": is_error}))
                }
                Part::Image { media_type, data_b64 } => {
                    blocks.push(json!({"type": "image", "source": {"type": "base64", "media_type": media_type, "data": data_b64}}))
                }
            }
        }
        if blocks.is_empty() {
            continue;
        }
        // Consecutive turns of the same role are merged (the API expects alternation).
        match messages.last_mut() {
            Some(prev) if prev["role"] == role => prev["content"]
                .as_array_mut()
                .expect("content array")
                .extend(blocks),
            _ => messages.push(json!({"role": role, "content": blocks})),
        }
    }
    // Automatic breakpoint on the last block of the last message (5 minute TTL: shorter TTLs come last).
    if let Some(last) = messages
        .last_mut()
        .and_then(|m| m["content"].as_array_mut())
        .and_then(|c| c.last_mut())
    {
        last["cache_control"] = json!({"type": "ephemeral"});
    }
    let mut body = json!({
        "model": req.model,
        "max_tokens": req.max_tokens,
        "stream": true,
        "messages": messages,
    });
    if !system.is_empty() {
        body["system"] = Value::Array(system);
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    match thinking_mode(&req.model) {
        ThinkingMode::Adaptive { xhigh: _ } => {
            body["thinking"] = json!({"type": "adaptive", "display": "summarized"});
            body["output_config"] = json!({"effort": req.effort.as_str()});
        }
        ThinkingMode::Budget { effort } => {
            body["thinking"] =
                json!({"type": "enabled", "budget_tokens": budget(req.effort, req.max_tokens)});
            if effort {
                let e = if req.effort == Effort::Max {
                    "high"
                } else {
                    req.effort.as_str()
                };
                body["output_config"] = json!({"effort": e});
            }
        }
    }
    body
}

#[derive(Debug)]
enum Block {
    Text(String),
    Thinking {
        text: String,
        signature: String,
    },
    Redacted(String),
    Tool {
        id: String,
        name: String,
        json: String,
    },
}

/// Incremental SSE state of one response.
#[derive(Default)]
pub struct StreamState {
    blocks: BTreeMap<u64, Block>,
    usage: Usage,
    stop: Option<StopReason>,
    model: String,
    done: bool,
}

impl StreamState {
    /// Applies one SSE event; returns `true` at `message_stop`.
    pub fn apply(
        &mut self,
        ev: &SseEvent,
        emit: &(dyn Fn(ChatEvent) + Send + Sync),
    ) -> Result<bool, LlmError> {
        if ev.data.is_empty() {
            return Ok(false);
        }
        let v: Value = serde_json::from_str(&ev.data)
            .map_err(|e| LlmError::Decode(format!("{e}: {}", ev.data)))?;
        let idx = v["index"].as_u64().unwrap_or(0);
        match v["type"].as_str().unwrap_or("") {
            "message_start" => {
                self.model = v["message"]["model"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                self.read_usage(&v["message"]["usage"]);
            }
            "content_block_start" => {
                let cb = &v["content_block"];
                let b = match cb["type"].as_str().unwrap_or("") {
                    "text" => Block::Text(cb["text"].as_str().unwrap_or_default().to_owned()),
                    "thinking" => Block::Thinking {
                        text: cb["thinking"].as_str().unwrap_or_default().to_owned(),
                        signature: cb["signature"].as_str().unwrap_or_default().to_owned(),
                    },
                    "redacted_thinking" => {
                        Block::Redacted(cb["data"].as_str().unwrap_or_default().to_owned())
                    }
                    "tool_use" => {
                        let id = cb["id"].as_str().unwrap_or_default().to_owned();
                        let name = cb["name"].as_str().unwrap_or_default().to_owned();
                        emit(ChatEvent::ToolCallStarted {
                            id: id.clone(),
                            name: name.clone(),
                        });
                        Block::Tool {
                            id,
                            name,
                            json: String::new(),
                        }
                    }
                    // Unknown block types (server tools, future additions) are ignored.
                    _ => return Ok(false),
                };
                self.blocks.insert(idx, b);
            }
            "content_block_delta" => {
                let d = &v["delta"];
                match (self.blocks.get_mut(&idx), d["type"].as_str().unwrap_or("")) {
                    (Some(Block::Text(t)), "text_delta") => {
                        let s = d["text"].as_str().unwrap_or_default();
                        t.push_str(s);
                        emit(ChatEvent::TextDelta(s.to_owned()));
                    }
                    (Some(Block::Thinking { text, .. }), "thinking_delta") => {
                        let s = d["thinking"].as_str().unwrap_or_default();
                        text.push_str(s);
                        emit(ChatEvent::ThinkingDelta(s.to_owned()));
                    }
                    (Some(Block::Thinking { signature, .. }), "signature_delta") => {
                        signature.push_str(d["signature"].as_str().unwrap_or_default());
                    }
                    (Some(Block::Tool { json, .. }), "input_json_delta") => {
                        json.push_str(d["partial_json"].as_str().unwrap_or_default())
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let Some(Block::Tool { id, name, json }) = self.blocks.get(&idx) {
                    emit(ChatEvent::ToolCall(ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        args: parse_args(json),
                    }));
                }
            }
            "message_delta" => {
                if let Some(s) = v["delta"]["stop_reason"].as_str() {
                    self.stop = Some(match s {
                        "end_turn" | "stop_sequence" => StopReason::EndTurn,
                        "tool_use" => StopReason::ToolUse,
                        "max_tokens" => StopReason::MaxTokens,
                        "refusal" => StopReason::Refusal,
                        "pause_turn" => StopReason::PauseTurn,
                        o => StopReason::Other(o.to_owned()),
                    });
                }
                self.read_usage(&v["usage"]);
            }
            "message_stop" => {
                self.done = true;
                return Ok(true);
            }
            "error" => {
                let kind = v["error"]["type"].as_str().unwrap_or("error").to_owned();
                let message = v["error"]["message"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                let retryable = matches!(
                    kind.as_str(),
                    "overloaded_error" | "api_error" | "rate_limit_error"
                );
                return Err(LlmError::Stream {
                    kind,
                    message,
                    retryable,
                });
            }
            _ => {} // ping and future event types
        }
        Ok(false)
    }

    fn read_usage(&mut self, u: &Value) {
        let get = |k: &str| u[k].as_u64();
        if let Some(n) = get("input_tokens") {
            self.usage.input = n;
        }
        if let Some(n) = get("output_tokens") {
            self.usage.output = n;
        }
        if let Some(n) = get("cache_read_input_tokens") {
            self.usage.cache_read = n;
        }
        if let Some(n) = get("cache_creation_input_tokens") {
            self.usage.cache_write = n;
        }
    }

    pub fn finish(self) -> Result<ChatResponse, LlmError> {
        if !self.done {
            return Err(LlmError::StreamDropped("message_stop not received".into()));
        }
        let mut parts = Vec::new();
        for (_, b) in self.blocks {
            parts.push(match b {
                Block::Text(text) => Part::Text { text },
                Block::Thinking { text, signature } => Part::Thinking {
                    text,
                    signature: Some(signature),
                    provider: KIND.into(),
                    redacted: false,
                },
                Block::Redacted(data) => Part::Thinking {
                    text: String::new(),
                    signature: Some(data),
                    provider: KIND.into(),
                    redacted: true,
                },
                Block::Tool { id, name, json } => Part::ToolCall {
                    id,
                    name,
                    args: parse_args(&json),
                    signature: None,
                },
            });
        }
        let stop = self.stop.unwrap_or(StopReason::EndTurn);
        Ok(ChatResponse {
            message: Message {
                role: Role::Assistant,
                parts,
            },
            stop,
            usage: self.usage,
            model: self.model,
        })
    }
}

/// Tool input as streamed (eager input streaming): invalid or truncated JSON is preserved so the harness can return
/// a structured "invalid JSON" error instead of running the tool.
pub fn parse_args(json: &str) -> Value {
    if json.trim().is_empty() {
        return json!({});
    }
    serde_json::from_str(json).unwrap_or_else(|_| json!({ crate::INVALID_JSON_KEY: json }))
}

#[async_trait]
impl Provider for Anthropic {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &'static str {
        KIND
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError> {
        let v = self
            .http
            .get_json(
                &format!("{}/v1/models?limit=100", self.base_url),
                &self.headers(),
                map_error,
            )
            .await?;
        Ok(v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| ModelInfo {
                id: m["id"].as_str().unwrap_or_default().to_owned(),
                display_name: m["display_name"].as_str().map(str::to_owned),
                context_window: m["max_input_tokens"].as_u64(),
                max_output: m["max_tokens"].as_u64(),
                supports_tools: Some(true),
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
        self.http
            .post_sse(
                &format!("{}/v1/messages", self.base_url),
                &self.headers(),
                &body,
                map_error,
                |ev| st.apply(&ev, emit),
            )
            .await?;
        let r = st.finish()?;
        emit(ChatEvent::Usage(r.usage));
        emit(ChatEvent::Done);
        Ok(r)
    }
}
