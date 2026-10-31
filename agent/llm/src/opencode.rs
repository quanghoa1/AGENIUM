//! OpenCode Zen / Go relay (opencode.ai): one key, several wire protocols. The model id decides the transport, as
//! the relay documents and Hermes agent implements: Anthropic Messages for `claude-*`, `minimax-*`, `qwen*`, `union-alpha`;
//! OpenAI Responses for `gpt-*`, `grok-*`, `muse-spark*`; chat/completions otherwise.
//! Every request carries `x-opencode-session` (Go rejects requests without it).

use async_trait::async_trait;

use crate::anthropic::Anthropic;
use crate::error::LlmError;
use crate::http::Http;
use crate::openai::OpenAiCompat;
use crate::responses::Responses;
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

/// One account's key works on both endpoints; the endpoint decides the bill. Zen answers 402 "Insufficient account
/// funds" to an account with a Go subscription and no credit (2026-10-31): the message says where the subscription is.
fn zen_hint(zen: bool, e: LlmError) -> LlmError {
    match e {
        LlmError::Http {
            status: 402,
            kind,
            message,
            quota,
            retry_after,
        } if zen => LlmError::Http {
            status: 402,
            kind,
            message: format!(
                "{message} (OpenCode Zen is paid from your account's credit; an OpenCode Go subscription is used \
                 through the OpenCode Go provider, https://opencode.ai/zen/go/v1, with the same key)"
            ),
            quota,
            retry_after,
        },
        e => e,
    }
}

pub fn is_opencode(base_url: &str) -> bool {
    base_url.to_ascii_lowercase().contains("opencode.ai")
}

pub struct OpenCode {
    id: String,
    /// The pay-as-you-go endpoint (Zen, `…/zen/v1`), not the subscription's (Go, `…/zen/go/v1`).
    zen: bool,
    anthropic: Anthropic,
    chat: OpenAiCompat,
    responses: Responses,
}

impl OpenCode {
    /// `base_url` is the chat/completions base, e.g. `https://opencode.ai/zen/go/v1`.
    pub fn new(id: &str, base_url: &str, api_key: &str, http: Http) -> Self {
        let chat_base = base_url.trim_end_matches('/');
        let messages_base = chat_base.strip_suffix("/v1").unwrap_or(chat_base);
        Self {
            id: id.to_owned(),
            zen: !chat_base.to_ascii_lowercase().contains("/zen/go"),
            anthropic: Anthropic::new(id, Some(messages_base), api_key, http.clone()),
            responses: Responses::new(id, chat_base, Some(api_key), http.clone(), true),
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
        let r = match wire_for(&req.model) {
            Wire::Anthropic => self.anthropic.chat(req, emit).await,
            Wire::ChatCompletions => self.chat.chat(req, emit).await,
            Wire::Responses => self.responses.chat(req, emit).await,
        };
        r.map_err(|e| zen_hint(self.zen, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(status: u16) -> LlmError {
        LlmError::Http {
            status,
            kind: "server_error".into(),
            message: "Upstream request failed: Insufficient account funds".into(),
            quota: false,
            retry_after: None,
        }
    }

    /// Zen's "no credit" says where a Go subscription is used; Go's answers and Zen's other errors are left as they are.
    #[test]
    fn zen_without_credit_points_to_go() {
        let msg = |e: LlmError| match e {
            LlmError::Http { message, .. } => message,
            e => e.to_string(),
        };
        assert!(msg(zen_hint(true, http(402))).contains("through the OpenCode Go provider"));
        assert!(!msg(zen_hint(false, http(402))).contains("OpenCode Go provider"));
        assert!(!msg(zen_hint(true, http(429))).contains("OpenCode Go provider"));
        let h = || {
            Http::new(
                std::time::Duration::from_secs(5),
                std::time::Duration::from_secs(5),
            )
            .unwrap()
        };
        let go = OpenCode::new("g", "https://opencode.ai/zen/go/v1", "k", h());
        let zen = OpenCode::new("z", "https://opencode.ai/zen/v1/", "k", h());
        assert!(!go.zen && zen.zen);
    }
}
