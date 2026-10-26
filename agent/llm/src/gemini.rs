//! Google Gemini adapter (`generateContent`, streamed with `alt=sse`). Thinking via `thinkingConfig`
//! (`thinkingLevel` on Gemini 3+, `thinkingBudget` on 2.5); thought signatures are replayed on the parts that carried
//! them. Caching: implicit prefix caching only (explicit `cachedContents` is off by default, decision 13).

use agenium_core::Effort;
use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde_json::{json, Value};

use crate::error::{parse_retry_after, LlmError};
use crate::http::{header, Http};
use crate::sse::SseEvent;
use crate::types::*;
use crate::Provider;

pub const KIND: &str = "gemini";

pub struct Gemini {
    id: String,
    base_url: String,
    api_key: String,
    http: Http,
}

impl Gemini {
    pub fn new(id: &str, base_url: Option<&str>, api_key: &str, http: Http) -> Self {
        Self {
            id: id.to_owned(),
            base_url: base_url
                .unwrap_or("https://generativelanguage.googleapis.com")
                .trim_end_matches('/')
                .to_owned(),
            api_key: api_key.to_owned(),
            http,
        }
    }

    fn headers(&self) -> Vec<(&'static str, String)> {
        vec![("x-goog-api-key", self.api_key.clone())]
    }
}

pub fn map_error(status: u16, h: &HeaderMap, body: &str) -> LlmError {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let kind = v["error"]["status"].as_str().unwrap_or("").to_owned();
    let message = v["error"]["message"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| body.chars().take(500).collect());
    let low = message.to_ascii_lowercase();
    // 429 RESOURCE_EXHAUSTED covers both per-minute limits and exhausted billing; only the latter is quota.
    let quota = status == 402
        || low.contains("billing")
        || low.contains("prepayment")
        || low.contains("credits");
    let retry_after = parse_retry_after(header(h, "retry-after").as_deref()).or_else(|| {
        v["error"]["details"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|d| {
                d["retryDelay"]
                    .as_str()
                    .and_then(|s| s.trim_end_matches('s').parse::<f64>().ok())
                    .map(std::time::Duration::from_secs_f64)
            })
    });
    LlmError::Http {
        status,
        kind,
        message,
        quota,
        retry_after,
    }
}

/// Gemini accepts an OpenAPI-like subset of JSON Schema: drop keywords it rejects and turn nullable unions into
/// `nullable`.
pub fn sanitize_schema(v: &Value) -> Value {
    match v {
        Value::Object(o) => {
            if let Some(any) = o.get("anyOf").and_then(Value::as_array) {
                let non_null: Vec<&Value> = any.iter().filter(|s| s["type"] != "null").collect();
                if non_null.len() == 1 && any.len() == 2 {
                    let mut inner = sanitize_schema(non_null[0]);
                    inner["nullable"] = json!(true);
                    if let Some(d) = o.get("description") {
                        inner["description"] = d.clone();
                    }
                    return inner;
                }
            }
            let mut out = serde_json::Map::new();
            for (k, val) in o {
                match k.as_str() {
                    "additionalProperties"
                    | "$schema"
                    | "$id"
                    | "default"
                    | "format"
                    | "title"
                    | "examples"
                    | "const" => {}
                    "type" if val.is_array() => {
                        let types: Vec<&Value> = val
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter(|t| *t != "null")
                            .collect();
                        if let Some(t) = types.first() {
                            out.insert("type".into(), (*t).clone());
                        }
                        if types.len() < val.as_array().unwrap().len() {
                            out.insert("nullable".into(), json!(true));
                        }
                    }
                    _ => {
                        out.insert(k.clone(), sanitize_schema(val));
                    }
                }
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(sanitize_schema).collect()),
        other => other.clone(),
    }
}

/// Gemini 2.x thinks within a budget (Agenium's budgets; `max` = dynamic); Gemini 3 and later take the model's own
/// `thinkingLevel` (ADR-114), none for a model whose levels are unknown.
fn thinking_config(model: &str, e: &Effort) -> Value {
    let m = model.to_ascii_lowercase();
    if m.contains("gemini-2.5") || m.contains("gemini-2.0") {
        let budget = match crate::effort::level_for(model, e)
            .unwrap_or_default()
            .as_str()
        {
            "low" => 1024,
            "high" => 24_576,
            "max" => -1,
            _ => 8192,
        };
        json!({"thinkingBudget": budget, "includeThoughts": true})
    } else {
        match crate::effort::level_for(model, e) {
            Some(l) => json!({"thinkingLevel": l.as_str(), "includeThoughts": true}),
            None => json!({"includeThoughts": true}),
        }
    }
}

pub fn build_body(req: &ChatRequest) -> Value {
    let mut contents: Vec<Value> = Vec::new();
    for m in &req.messages {
        let role = match m.role {
            Role::User => "user",
            Role::Assistant => "model",
        };
        let mut parts: Vec<Value> = Vec::new();
        for p in &m.parts {
            match p {
                Part::Text { text } if !text.is_empty() => parts.push(json!({"text": text})),
                Part::Thinking {
                    text,
                    signature,
                    provider,
                    ..
                } if provider == KIND && m.role == Role::Assistant => {
                    let mut t = json!({"text": text, "thought": true});
                    if let Some(s) = signature {
                        t["thoughtSignature"] = json!(s);
                    }
                    parts.push(t);
                }
                Part::ToolCall {
                    name,
                    args,
                    signature,
                    ..
                } => {
                    let mut c = json!({"functionCall": {"name": name, "args": args}});
                    if let Some(s) = signature {
                        c["thoughtSignature"] = json!(s);
                    }
                    parts.push(c);
                }
                Part::ToolResult {
                    name,
                    content,
                    is_error,
                    ..
                } => {
                    let response = if *is_error {
                        json!({"error": content})
                    } else {
                        json!({"result": content})
                    };
                    parts.push(json!({"functionResponse": {"name": name, "response": response}}));
                }
                Part::Image {
                    media_type,
                    data_b64,
                } => parts.push(json!({"inlineData": {"mimeType": media_type, "data": data_b64}})),
                _ => {}
            }
        }
        if parts.is_empty() {
            continue;
        }
        match contents.last_mut() {
            Some(prev) if prev["role"] == role => {
                prev["parts"].as_array_mut().expect("parts").extend(parts)
            }
            _ => contents.push(json!({"role": role, "parts": parts})),
        }
    }
    let mut body = json!({
        "contents": contents,
        "generationConfig": {"maxOutputTokens": req.max_tokens, "thinkingConfig": thinking_config(&req.model, &req.effort)},
    });
    let system: Vec<Value> = req
        .system
        .iter()
        .filter(|b| !b.text.is_empty())
        .map(|b| json!({"text": b.text}))
        .collect();
    if !system.is_empty() {
        body["systemInstruction"] = json!({"parts": system});
    }
    if !req.tools.is_empty() {
        let decls: Vec<Value> = req
            .tools
            .iter()
            .map(|t| json!({"name": t.name, "description": t.description, "parameters": sanitize_schema(&t.parameters)}))
            .collect();
        body["tools"] = json!([{"functionDeclarations": decls}]);
        if req.tools_off {
            body["toolConfig"] = json!({"functionCallingConfig": {"mode": "NONE"}});
        }
    }
    body
}

#[derive(Default)]
pub struct StreamState {
    parts: Vec<Part>,
    finish: Option<StopReason>,
    usage: Usage,
    model: String,
    calls: u32,
}

impl StreamState {
    pub fn apply(
        &mut self,
        ev: &SseEvent,
        emit: &(dyn Fn(ChatEvent) + Send + Sync),
    ) -> Result<bool, LlmError> {
        if ev.data.trim().is_empty() {
            return Ok(false);
        }
        let v: Value = serde_json::from_str(&ev.data)
            .map_err(|e| LlmError::Decode(format!("{e}: {}", ev.data)))?;
        if v["error"].is_object() {
            let kind = v["error"]["status"].as_str().unwrap_or("error").to_owned();
            return Err(LlmError::Stream {
                retryable: kind == "UNAVAILABLE" || kind == "INTERNAL",
                kind,
                message: v["error"]["message"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
        if let Some(m) = v["modelVersion"].as_str() {
            self.model = m.to_owned();
        }
        if let Some(c) = v["candidates"].get(0) {
            for p in c["content"]["parts"].as_array().into_iter().flatten() {
                let sig = p["thoughtSignature"].as_str().map(str::to_owned);
                if let Some(fc) = p.get("functionCall") {
                    self.calls += 1;
                    let id = fc["id"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("gemini_call_{}", self.calls));
                    let name = fc["name"].as_str().unwrap_or_default().to_owned();
                    let args = fc["args"].clone();
                    let args = if args.is_null() { json!({}) } else { args };
                    emit(ChatEvent::ToolCall(ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        args: args.clone(),
                    }));
                    self.parts.push(Part::ToolCall {
                        id,
                        name,
                        args,
                        signature: sig,
                    });
                } else if let Some(t) = p["text"].as_str() {
                    let thought = p["thought"].as_bool().unwrap_or(false);
                    if thought {
                        emit(ChatEvent::ThinkingDelta(t.to_owned()));
                        match self.parts.last_mut() {
                            Some(Part::Thinking {
                                text, signature, ..
                            }) => {
                                text.push_str(t);
                                if sig.is_some() {
                                    *signature = sig;
                                }
                            }
                            _ => self.parts.push(Part::Thinking {
                                text: t.to_owned(),
                                signature: sig,
                                provider: KIND.into(),
                                redacted: false,
                                model: None,
                            }),
                        }
                    } else {
                        emit(ChatEvent::TextDelta(t.to_owned()));
                        match self.parts.last_mut() {
                            Some(Part::Text { text }) => text.push_str(t),
                            _ => self.parts.push(Part::Text { text: t.to_owned() }),
                        }
                        if let Some(s) = sig {
                            // A signature on a text part is kept on a zero-length thought so it can be replayed.
                            self.parts.push(Part::Thinking {
                                text: String::new(),
                                signature: Some(s),
                                provider: KIND.into(),
                                redacted: false,
                                model: None,
                            });
                        }
                    }
                }
            }
            if let Some(f) = c["finishReason"].as_str() {
                self.finish = Some(match f {
                    "STOP" => StopReason::EndTurn,
                    "MAX_TOKENS" => StopReason::MaxTokens,
                    "SAFETY" | "RECITATION" | "PROHIBITED_CONTENT" | "BLOCKLIST" | "SPII" => {
                        StopReason::Refusal
                    }
                    o => StopReason::Other(o.to_owned()),
                });
            }
        }
        let u = &v["usageMetadata"];
        if u.is_object() {
            let prompt = u["promptTokenCount"].as_u64().unwrap_or(0);
            let cached = u["cachedContentTokenCount"].as_u64().unwrap_or(0);
            let thoughts = u["thoughtsTokenCount"].as_u64().unwrap_or(0);
            self.usage = Usage {
                input: prompt.saturating_sub(cached),
                output: u["candidatesTokenCount"].as_u64().unwrap_or(0) + thoughts,
                cache_read: cached,
                cache_write: 0,
                cache_write_1h: 0,
                reasoning: thoughts,
            };
        }
        Ok(false)
    }

    pub fn finish(self) -> Result<ChatResponse, LlmError> {
        if self.finish.is_none() {
            return Err(LlmError::StreamDropped(
                "stream ended without finishReason".into(),
            ));
        }
        let has_calls = self
            .parts
            .iter()
            .any(|p| matches!(p, Part::ToolCall { .. }));
        let stop = match self.finish {
            Some(StopReason::EndTurn) if has_calls => StopReason::ToolUse,
            Some(s) => s,
            None => StopReason::EndTurn,
        };
        Ok(ChatResponse {
            message: Message {
                role: Role::Assistant,
                parts: self.parts,
            },
            stop,
            usage: self.usage,
            model: self.model,
            dropped_thinking: 0,
        })
    }
}

#[async_trait]
impl Provider for Gemini {
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
                &format!("{}/v1beta/models?pageSize=200", self.base_url),
                &self.headers(),
                map_error,
            )
            .await?;
        Ok(v["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| {
                m["supportedGenerationMethods"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|x| x == "generateContent"))
            })
            .map(|m| ModelInfo {
                id: m["name"]
                    .as_str()
                    .unwrap_or_default()
                    .trim_start_matches("models/")
                    .to_owned(),
                display_name: m["displayName"].as_str().map(str::to_owned),
                context_window: m["inputTokenLimit"].as_u64(),
                max_output: m["outputTokenLimit"].as_u64(),
                supports_tools: Some(true),
                supports_vision: Some(true),
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
        let url = format!(
            "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
            self.base_url, req.model
        );
        let mut st = StreamState::default();
        let r = self
            .http
            .post_sse(&url, &self.headers(), &body, map_error, |ev| {
                st.apply(&ev, emit)
            })
            .await;
        match r {
            Ok(()) => {}
            // Gemini ends the stream by closing it after the chunk carrying finishReason.
            Err(LlmError::StreamDropped(_)) if st.finish.is_some() => {}
            Err(e) => return Err(e),
        }
        let resp = st.finish()?;
        emit(ChatEvent::Usage(resp.usage));
        emit(ChatEvent::Done);
        Ok(resp)
    }
}
