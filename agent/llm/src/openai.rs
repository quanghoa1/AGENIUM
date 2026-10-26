//! OpenAI-compatible Chat Completions adapter, shared by OpenAI, OpenRouter, Opencode, DeepSeek and Local/custom
//! endpoints (Ollama, LM Studio, llama.cpp server, vLLM; decision 31). Streaming SSE with `[DONE]`; tool calls are
//! assembled from indexed deltas; usage normalised from `prompt_tokens_details.cached_tokens` (OpenAI) and
//! `prompt_cache_hit_tokens` (DeepSeek). Caching is automatic prefix matching: keep the prefix stable and pass
//! `prompt_cache_key` where supported.

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

pub const KIND: &str = "openai_compat";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    OpenAi,
    DeepSeek,
    OpenRouter,
    /// OpenCode Zen / Go relay (opencode.ai): requires a per-conversation `x-opencode-session` affinity header (Go
    /// rejects requests without it; it keeps the upstream prompt cache warm). Serves several model families.
    OpenCode,
    /// Local servers and anything else speaking the protocol.
    Generic,
}

impl Flavor {
    pub fn detect(base_url: &str) -> Flavor {
        let u = base_url.to_ascii_lowercase();
        if u.contains("api.openai.com") {
            Flavor::OpenAi
        } else if u.contains("deepseek.com") {
            Flavor::DeepSeek
        } else if u.contains("openrouter.ai") {
            Flavor::OpenRouter
        } else if u.contains("opencode.ai") {
            Flavor::OpenCode
        } else {
            Flavor::Generic
        }
    }
}

/// Reasoning models take `reasoning_effort`; others reject it.
fn is_reasoning_model(flavor: Flavor, model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    match flavor {
        Flavor::OpenAi => m.starts_with('o') || m.starts_with("gpt-5"),
        Flavor::DeepSeek => m.contains("reasoner") || m.contains("v3.2") || m.contains("v4"),
        Flavor::OpenRouter => true,
        Flavor::OpenCode => m.starts_with("deepseek"),
        Flavor::Generic => false,
    }
}

/// Effort mapping of the architecture table: OpenAI low/medium/high/xhigh(max), DeepSeek low/high/high/max.
pub fn effort_value(flavor: Flavor, e: Effort) -> &'static str {
    // DeepSeek models behind OpenCode take DeepSeek's levels.
    let flavor = if flavor == Flavor::OpenCode {
        Flavor::DeepSeek
    } else {
        flavor
    };
    match (flavor, e) {
        (_, Effort::Low) => "low",
        (Flavor::DeepSeek, Effort::Medium | Effort::High) => "high",
        (Flavor::DeepSeek, Effort::Max) => "max",
        (_, Effort::Medium) => "medium",
        (_, Effort::High) => "high",
        (Flavor::OpenAi, Effort::Max) => "xhigh",
        (_, Effort::Max) => "high",
    }
}

pub struct OpenAiCompat {
    id: String,
    base_url: String,
    api_key: Option<String>,
    flavor: Flavor,
    http: Http,
}

impl OpenAiCompat {
    pub fn new(id: &str, base_url: &str, api_key: Option<&str>, http: Http) -> Self {
        let base = base_url.trim_end_matches('/').to_owned();
        Self {
            id: id.to_owned(),
            flavor: Flavor::detect(&base),
            base_url: base,
            api_key: api_key.filter(|k| !k.is_empty()).map(str::to_owned),
            http,
        }
    }

    /// `GET /models` as the server reports it (OpenRouter adds `context_length`).
    async fn list_openai_models(&self) -> Result<Vec<ModelInfo>, LlmError> {
        let v = self
            .http
            .get_json(
                &format!("{}/models", self.base_url),
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
                display_name: m["name"].as_str().map(str::to_owned),
                context_window: m["context_length"].as_u64(),
                max_output: None,
                supports_tools: None,
                supports_vision: None,
            })
            .collect())
    }

    fn headers(&self) -> Vec<(&'static str, String)> {
        let mut h = Vec::new();
        if let Some(k) = &self.api_key {
            h.push(("authorization", format!("Bearer {k}")));
        }
        if self.flavor == Flavor::OpenRouter {
            h.push(("x-title", "Agenium".into()));
        }
        h
    }

    fn request_headers(&self, req: &ChatRequest) -> Vec<(&'static str, String)> {
        let mut h = self.headers();
        if self.flavor == Flavor::OpenCode {
            let key = req
                .cache_key
                .clone()
                .unwrap_or_else(|| format!("oneshot-{:016x}", rand::random::<u64>()));
            h.push(("x-opencode-session", key));
        }
        h
    }
}

/// Conversation affinity key for relays (stable per conversation; one-shot requests get a random key).
pub fn session_key(req: &ChatRequest) -> String {
    req.cache_key
        .clone()
        .unwrap_or_else(|| format!("oneshot-{:016x}", rand::random::<u64>()))
}

pub fn map_error(status: u16, h: &HeaderMap, body: &str) -> LlmError {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let err = if v["error"].is_object() {
        &v["error"]
    } else {
        &v
    };
    let code = err["code"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| err["code"].as_i64().map(|c| c.to_string()));
    let kind = code
        .clone()
        .or_else(|| err["type"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let message = err["message"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| body.chars().take(500).collect());
    let low = message.to_ascii_lowercase();
    let quota = status == 402
        || kind == "insufficient_quota"
        || low.contains("insufficient balance")
        || low.contains("insufficient credits")
        || low.contains("exceeded your current quota");
    LlmError::Http {
        status,
        kind,
        message,
        quota,
        retry_after: parse_retry_after(header(h, "retry-after").as_deref()),
    }
}

pub fn build_body(flavor: Flavor, req: &ChatRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    let system: Vec<&str> = req
        .system
        .iter()
        .map(|b| b.text.as_str())
        .filter(|t| !t.is_empty())
        .collect();
    if !system.is_empty() {
        messages.push(json!({"role": "system", "content": system.join("\n\n")}));
    }
    for m in &req.messages {
        match m.role {
            Role::User => {
                // Tool results first (they must directly follow the assistant tool_calls message).
                for p in &m.parts {
                    if let Part::ToolResult {
                        call_id,
                        content,
                        is_error,
                        ..
                    } = p
                    {
                        let c = if *is_error {
                            format!("ERROR: {content}")
                        } else {
                            content.clone()
                        };
                        messages
                            .push(json!({"role": "tool", "tool_call_id": call_id, "content": c}));
                    }
                }
                let mut content: Vec<Value> = Vec::new();
                for p in &m.parts {
                    match p {
                        Part::Text { text } if !text.is_empty() => content.push(json!({"type": "text", "text": text})),
                        Part::Image { media_type, data_b64 } => {
                            content.push(json!({"type": "image_url", "image_url": {"url": format!("data:{media_type};base64,{data_b64}")}}))
                        }
                        _ => {}
                    }
                }
                if content.len() == 1 && content[0]["type"] == "text" {
                    messages.push(json!({"role": "user", "content": content[0]["text"]}));
                } else if !content.is_empty() {
                    messages.push(json!({"role": "user", "content": content}));
                }
            }
            Role::Assistant => {
                let text = m.text();
                let calls: Vec<Value> = m
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::ToolCall { id, name, args, .. } => Some(json!({
                            "id": id, "type": "function",
                            "function": {"name": name, "arguments": args.to_string()}
                        })),
                        _ => None,
                    })
                    .collect();
                let mut msg = json!({"role": "assistant", "content": if text.is_empty() { Value::Null } else { Value::String(text) }});
                if !calls.is_empty() {
                    msg["tool_calls"] = Value::Array(calls);
                }
                if flavor == Flavor::DeepSeek
                    || (flavor == Flavor::OpenCode && req.model.starts_with("deepseek"))
                {
                    // DeepSeek thinking mode expects reasoning_content back within a tool-calling turn.
                    let r: String = m
                        .parts
                        .iter()
                        .filter_map(|p| match p {
                            Part::Thinking { text, provider, .. } if provider == KIND => {
                                Some(text.as_str())
                            }
                            _ => None,
                        })
                        .collect();
                    if !r.is_empty() {
                        msg["reasoning_content"] = Value::String(r);
                    }
                }
                messages.push(msg);
            }
        }
    }
    let mut body = json!({
        "model": req.model,
        "stream": true,
        "messages": messages,
    });
    if flavor != Flavor::Generic {
        body["stream_options"] = json!({"include_usage": true});
    }
    if flavor == Flavor::OpenAi {
        body["max_completion_tokens"] = json!(req.max_tokens);
        if let Some(k) = &req.cache_key {
            body["prompt_cache_key"] = json!(k);
        }
    } else {
        body["max_tokens"] = json!(req.max_tokens);
    }
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(
            req.tools
                .iter()
                .map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.parameters}}))
                .collect(),
        );
    }
    if is_reasoning_model(flavor, &req.model) {
        let e = effort_value(flavor, req.effort);
        if flavor == Flavor::OpenRouter {
            body["reasoning"] = json!({"effort": e});
        } else {
            body["reasoning_effort"] = json!(e);
        }
    }
    body
}

#[derive(Default)]
pub struct StreamState {
    text: String,
    reasoning: String,
    calls: BTreeMap<u64, (String, String, String)>,
    finish: Option<StopReason>,
    usage: Usage,
    model: String,
    done: bool,
}

impl StreamState {
    pub fn apply(
        &mut self,
        ev: &SseEvent,
        emit: &(dyn Fn(ChatEvent) + Send + Sync),
    ) -> Result<bool, LlmError> {
        let data = ev.data.trim();
        if data.is_empty() {
            return Ok(false);
        }
        if data == "[DONE]" {
            self.done = true;
            return Ok(true);
        }
        let v: Value =
            serde_json::from_str(data).map_err(|e| LlmError::Decode(format!("{e}: {data}")))?;
        if v["error"].is_object() {
            let kind = v["error"]["code"]
                .as_str()
                .or(v["error"]["type"].as_str())
                .unwrap_or("error")
                .to_owned();
            let message = v["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            return Err(LlmError::Stream {
                retryable: kind.contains("overload") || kind.contains("server"),
                kind,
                message,
            });
        }
        if let Some(m) = v["model"].as_str() {
            self.model = m.to_owned();
        }
        if let Some(choice) = v["choices"].get(0) {
            let d = &choice["delta"];
            if let Some(s) = d["content"].as_str() {
                if !s.is_empty() {
                    self.text.push_str(s);
                    emit(ChatEvent::TextDelta(s.to_owned()));
                }
            }
            for key in ["reasoning_content", "reasoning"] {
                if let Some(s) = d[key].as_str() {
                    if !s.is_empty() {
                        self.reasoning.push_str(s);
                        emit(ChatEvent::ThinkingDelta(s.to_owned()));
                    }
                }
            }
            for tc in d["tool_calls"].as_array().into_iter().flatten() {
                let i = tc["index"].as_u64().unwrap_or(0);
                let e = self.calls.entry(i).or_default();
                if let Some(id) = tc["id"].as_str() {
                    e.0 = id.to_owned();
                }
                if let Some(n) = tc["function"]["name"].as_str() {
                    if e.1.is_empty() {
                        e.1 = n.to_owned();
                        emit(ChatEvent::ToolCallStarted {
                            id: e.0.clone(),
                            name: e.1.clone(),
                        });
                    }
                }
                if let Some(a) = tc["function"]["arguments"].as_str() {
                    e.2.push_str(a);
                }
            }
            if let Some(f) = choice["finish_reason"].as_str() {
                self.finish = Some(match f {
                    "stop" => StopReason::EndTurn,
                    "tool_calls" | "function_call" => StopReason::ToolUse,
                    "length" => StopReason::MaxTokens,
                    "content_filter" => StopReason::Refusal,
                    o => StopReason::Other(o.to_owned()),
                });
            }
        }
        let u = &v["usage"];
        if u.is_object() {
            let prompt = u["prompt_tokens"].as_u64().unwrap_or(0);
            let cached = u["prompt_tokens_details"]["cached_tokens"]
                .as_u64()
                .or(u["prompt_cache_hit_tokens"].as_u64())
                .unwrap_or(0);
            self.usage = Usage {
                input: prompt.saturating_sub(cached),
                output: u["completion_tokens"].as_u64().unwrap_or(0),
                cache_read: cached,
                cache_write: 0,
                reasoning: u["completion_tokens_details"]["reasoning_tokens"]
                    .as_u64()
                    .unwrap_or(0),
            };
        }
        Ok(false)
    }

    pub fn finish(mut self) -> Result<ChatResponse, LlmError> {
        if !self.done && self.finish.is_none() {
            return Err(LlmError::StreamDropped(
                "stream ended without finish_reason".into(),
            ));
        }
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
        for (i, (id, name, args)) in std::mem::take(&mut self.calls) {
            let id = if id.is_empty() {
                format!("call_{i}")
            } else {
                id
            };
            parts.push(Part::ToolCall {
                id,
                name,
                args: crate::anthropic::parse_args(&args),
                signature: None,
            });
        }
        let has_calls = parts.iter().any(|p| matches!(p, Part::ToolCall { .. }));
        let stop = match self.finish.take() {
            Some(StopReason::EndTurn) if has_calls => StopReason::ToolUse,
            Some(s) => s,
            None => {
                if has_calls {
                    StopReason::ToolUse
                } else {
                    StopReason::EndTurn
                }
            }
        };
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

#[async_trait]
impl Provider for OpenAiCompat {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &'static str {
        KIND
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError> {
        let mut models = self.list_openai_models().await?;
        if self.flavor == Flavor::Generic {
            crate::local::enrich(&self.http, &self.base_url, &mut models).await;
        }
        Ok(models)
    }

    async fn chat(
        &self,
        req: &ChatRequest,
        emit: &(dyn Fn(ChatEvent) + Send + Sync),
    ) -> Result<ChatResponse, LlmError> {
        let body = build_body(self.flavor, req);
        let mut st = StreamState::default();
        let r = self
            .http
            .post_sse(
                &format!("{}/chat/completions", self.base_url),
                &self.request_headers(req),
                &body,
                map_error,
                |ev| st.apply(&ev, emit),
            )
            .await;
        match r {
            Ok(()) => {}
            // Some servers close right after the final chunk without `[DONE]`.
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
