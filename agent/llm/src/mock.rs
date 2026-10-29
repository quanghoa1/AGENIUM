//! Scripted provider for tests and evals: each call pops the next responder, which may look at the request
//! (e.g. to answer based on the last tool result). Every request is recorded for assertions (record/replay).

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::LlmError;
use crate::types::*;
use crate::Provider;

pub type Responder = Box<dyn FnMut(&ChatRequest) -> Result<ChatResponse, LlmError> + Send>;

pub struct ScriptedProvider {
    id: String,
    script: Mutex<VecDeque<Responder>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedProvider {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            script: Mutex::new(VecDeque::new()),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn push(&self, r: Responder) -> &Self {
        self.script
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push_back(r);
        self
    }

    /// Next turn: these tool calls (`(name, args)`), with optional text before them.
    pub fn push_calls(&self, text: &str, calls: Vec<(&str, serde_json::Value)>) -> &Self {
        let resp = calls_response(text, calls);
        self.push(Box::new(move |_| Ok(resp.clone())))
    }

    pub fn push_text(&self, text: &str) -> &Self {
        let resp = ChatResponse {
            message: Message::assistant_text(text),
            stop: StopReason::EndTurn,
            usage: usage(),
            model: "scripted".into(),
            dropped_thinking: 0,
            refusal: None,
        };
        self.push(Box::new(move |_| Ok(resp.clone())))
    }

    pub fn push_error(&self, e: LlmError) -> &Self {
        self.push(Box::new(move |_| Err(e.clone())))
    }

    pub fn requests(&self) -> Vec<ChatRequest> {
        self.requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn remaining(&self) -> usize {
        self.script.lock().unwrap_or_else(|p| p.into_inner()).len()
    }
}

fn usage() -> Usage {
    Usage {
        input: 100,
        output: 20,
        cache_read: 0,
        cache_write: 0,
        cache_write_1h: 0,
        reasoning: 0,
    }
}

pub fn calls_response(text: &str, calls: Vec<(&str, serde_json::Value)>) -> ChatResponse {
    let mut parts = Vec::new();
    if !text.is_empty() {
        parts.push(Part::Text {
            text: text.to_owned(),
        });
    }
    for (i, (name, args)) in calls.into_iter().enumerate() {
        parts.push(Part::ToolCall {
            id: format!("call_{}_{}", name, i),
            name: name.to_owned(),
            args,
            signature: None,
        });
    }
    let stop = if parts.iter().any(|p| matches!(p, Part::ToolCall { .. })) {
        StopReason::ToolUse
    } else {
        StopReason::EndTurn
    };
    ChatResponse {
        message: Message {
            role: Role::Assistant,
            parts,
        },
        stop,
        usage: usage(),
        model: "scripted".into(),
        dropped_thinking: 0,
        refusal: None,
    }
}

#[async_trait]
impl Provider for ScriptedProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &'static str {
        "mock"
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError> {
        Ok(vec![ModelInfo {
            id: "scripted".into(),
            display_name: Some("Scripted test model".into()),
            context_window: Some(200_000),
            max_output: Some(8192),
            supports_tools: Some(true),
            supports_vision: None,
            efforts: None,
            default_effort: None,
        }])
    }

    async fn chat(
        &self,
        req: &ChatRequest,
        emit: &(dyn Fn(ChatEvent) + Send + Sync),
    ) -> Result<ChatResponse, LlmError> {
        self.requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(req.clone());
        let next = self
            .script
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop_front();
        let mut f =
            next.ok_or_else(|| LlmError::Config("scripted provider has no more responses".into()))?;
        let r = f(req)?;
        for p in &r.message.parts {
            match p {
                Part::Text { text } => emit(ChatEvent::TextDelta(text.clone())),
                Part::ToolCall { id, name, args, .. } => emit(ChatEvent::ToolCall(ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    args: args.clone(),
                })),
                _ => {}
            }
        }
        emit(ChatEvent::Usage(r.usage));
        emit(ChatEvent::Done);
        Ok(r)
    }
}
