//! Vendor API layer (decision 13): one `Provider` trait over a vendor-neutral message model; adapters for Anthropic
//! Messages, Gemini, OpenAI-compatible chat completions (OpenAI, OpenRouter, DeepSeek, the ADR-107 presets, local
//! servers), OpenAI Responses, the OpenCode relay (several wires behind one key) and TypeSafe's System One; and a
//! scripted provider for tests and evals. Retry/backoff, per-key concurrency limits and the display-only price table live here; the
//! crate knows nothing about tickets.

pub mod anthropic;
pub mod effort;
pub mod error;
pub mod gemini;
pub mod history;
pub mod http;
pub mod local;
pub mod mock;
pub mod openai;
pub mod opencode;
pub mod pricing;
pub mod responses;
pub mod retry;
pub mod sse;
pub mod systemone;
pub mod types;
pub mod vendor_usage;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Semaphore;

pub use error::{classify, is_context_overflow, Classified, LlmError};
pub use retry::{CallError, RetryPolicy};
pub use types::*;

/// Key used in tool-call arguments that could not be parsed as JSON (the harness answers with a structured error).
pub const INVALID_JSON_KEY: &str = "__invalid_json__";

#[async_trait]
pub trait Provider: Send + Sync {
    /// Provider id from configuration.
    fn id(&self) -> &str;
    /// Adapter kind (`anthropic`, `gemini`, `openai_compat`, `opencode`, `responses`, `mock`, ...): thinking blocks
    /// replay only within a kind (and only to the model that wrote them, ADR-101).
    fn kind(&self) -> &'static str;
    async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError>;
    /// Streams one model turn; `emit` receives live events, the returned response is authoritative.
    async fn chat(
        &self,
        req: &ChatRequest,
        emit: &(dyn Fn(ChatEvent) + Send + Sync),
    ) -> Result<ChatResponse, LlmError>;
}

/// Concurrency limiter per API key (architecture "Quyết định API bổ sung" #4).
pub struct Limited {
    inner: Arc<dyn Provider>,
    sem: Semaphore,
}

impl Limited {
    pub fn new(inner: Arc<dyn Provider>, max_concurrency: usize) -> Self {
        Self {
            inner,
            sem: Semaphore::new(max_concurrency.max(1)),
        }
    }
}

#[async_trait]
impl Provider for Limited {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn kind(&self) -> &'static str {
        self.inner.kind()
    }
    async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError> {
        self.inner.list_models().await
    }
    async fn chat(
        &self,
        req: &ChatRequest,
        emit: &(dyn Fn(ChatEvent) + Send + Sync),
    ) -> Result<ChatResponse, LlmError> {
        let _permit = self
            .sem
            .acquire()
            .await
            .map_err(|_| LlmError::Config("limiter closed".into()))?;
        self.inner.chat(req, emit).await
    }
}

/// One model turn with the retry policy. A dropped stream re-sends the identical request; the live view is told
/// to discard partial output (`ChatEvent::Restart`).
pub async fn chat_with_retry(
    p: &dyn Provider,
    req: &ChatRequest,
    policy: &RetryPolicy,
    emit: &(dyn Fn(ChatEvent) + Send + Sync),
    on_retry: impl FnMut(u32, Duration, &LlmError),
) -> Result<ChatResponse, CallError> {
    policy
        .run(
            |attempt| async move {
                if attempt > 0 {
                    emit(ChatEvent::Restart);
                }
                p.chat(req, emit).await
            },
            tokio::time::sleep,
            on_retry,
        )
        .await
}

/// Connection details for building a provider (from `config.toml` + the secret store).
#[derive(Debug, Clone)]
pub struct ProviderSpec {
    pub id: String,
    pub kind: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub max_concurrency: usize,
}

pub fn build_provider(spec: &ProviderSpec) -> Result<Arc<dyn Provider>, LlmError> {
    let http = http::Http::default_client()?;
    let key = spec.api_key.clone().unwrap_or_default();
    let p: Arc<dyn Provider> = match spec.kind.as_str() {
        "anthropic" => {
            if key.is_empty() {
                return Err(LlmError::Config(format!(
                    "provider {} needs an API key",
                    spec.id
                )));
            }
            Arc::new(anthropic::Anthropic::new(
                &spec.id,
                spec.base_url.as_deref(),
                &key,
                http,
            ))
        }
        "gemini" => {
            if key.is_empty() {
                return Err(LlmError::Config(format!(
                    "provider {} needs an API key",
                    spec.id
                )));
            }
            Arc::new(gemini::Gemini::new(
                &spec.id,
                spec.base_url.as_deref(),
                &key,
                http,
            ))
        }
        "openai_compat" => {
            let base = spec
                .base_url
                .as_deref()
                .ok_or_else(|| LlmError::Config(format!("provider {} needs base_url", spec.id)))?;
            if opencode::is_opencode(base) && !key.is_empty() {
                Arc::new(opencode::OpenCode::new(&spec.id, base, &key, http))
            } else {
                Arc::new(openai::OpenAiCompat::new(
                    &spec.id,
                    base,
                    spec.api_key.as_deref(),
                    http,
                ))
            }
        }
        other => return Err(LlmError::Config(format!("unknown provider kind {other:?}"))),
    };
    Ok(Arc::new(Limited::new(p, spec.max_concurrency)))
}
