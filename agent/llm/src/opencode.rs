//! OpenCode Zen / Go relay (opencode.ai): one key, several wire protocols. The model id decides the transport, as
//! the relay documents and Hermes agent implements: Anthropic Messages for `claude-*`, `minimax-*`, `qwen*`;
//! OpenAI Responses for `gpt-*`, `grok-*`, `muse-spark*` (not supported here yet); chat/completions otherwise.
//! Every request carries `x-opencode-session` (Go rejects requests without it).

use async_trait::async_trait;

use crate::anthropic::Anthropic;
use crate::error::LlmError;
use crate::http::Http;
use crate::openai::OpenAiCompat;
use crate::types::*;
use crate::Provider;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    Anthropic,
    Responses,
    ChatCompletions,
}

pub fn wire_for(model: &str) -> Wire {
    let m = model.to_ascii_lowercase();
    if ["claude-", "minimax-", "qwen", "union-alpha"]
        .iter()
        .any(|p| m.starts_with(p))
    {
        Wire::Anthropic
    } else if ["gpt-", "grok-", "muse-spark"]
        .iter()
        .any(|p| m.starts_with(p))
    {
        Wire::Responses
    } else {
        Wire::ChatCompletions
    }
}

pub fn is_opencode(base_url: &str) -> bool {
    base_url.to_ascii_lowercase().contains("opencode.ai")
}

pub struct OpenCode {
    id: String,
    anthropic: Anthropic,
    chat: OpenAiCompat,
}

impl OpenCode {
    /// `base_url` is the chat/completions base, e.g. `https://opencode.ai/zen/go/v1`.
    pub fn new(id: &str, base_url: &str, api_key: &str, http: Http) -> Self {
        let chat_base = base_url.trim_end_matches('/');
        let messages_base = chat_base.strip_suffix("/v1").unwrap_or(chat_base);
        Self {
            id: id.to_owned(),
            anthropic: Anthropic::new(id, Some(messages_base), api_key, http.clone()),
            chat: OpenAiCompat::new(id, chat_base, Some(api_key), http),
        }
    }
}

#[async_trait]
impl Provider for OpenCode {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &'static str {
        "opencode"
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError> {
        self.chat.list_models().await
    }

    async fn chat(
        &self,
        req: &ChatRequest,
        emit: &(dyn Fn(ChatEvent) + Send + Sync),
    ) -> Result<ChatResponse, LlmError> {
        match wire_for(&req.model) {
            Wire::Anthropic => self.anthropic.chat(req, emit).await,
            Wire::ChatCompletions => self.chat.chat(req, emit).await,
            Wire::Responses => Err(LlmError::Unsupported(format!(
                "{} on OpenCode only speaks the OpenAI Responses API, which Agenium does not support yet; pick a deepseek-, glm-, kimi-, claude-, qwen- or minimax- model",
                req.model
            ))),
        }
    }
}
