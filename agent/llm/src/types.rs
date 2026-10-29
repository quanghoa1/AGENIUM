//! Vendor-neutral message model (architecture "Kiểu thống nhất"). Run context is stored in this form so a run can
//! resume on another provider; vendor-specific thinking blocks are kept for replay to the same provider only.

use agenium_core::Effort;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Part {
    Text {
        text: String,
    },
    /// Reasoning returned by the provider. Replayed unchanged (signature included) to the provider that wrote it;
    /// dropped for any other provider. Never stored in memory (decision 13).
    Thinking {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        /// Provider kind that produced it (`anthropic`, `gemini`, `openai_compat`).
        provider: String,
        #[serde(default)]
        redacted: bool,
        /// Model that wrote it (set by the agent loop). A signed block is valid for that model only, so it is not
        /// replayed to another one (ADR-101); `None` (older history) replays as before.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    ToolCall {
        id: String,
        name: String,
        args: serde_json::Value,
        /// Gemini thought signature attached to the call part.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    ToolResult {
        call_id: String,
        name: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    Image {
        media_type: String,
        data_b64: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
}

impl Message {
    pub fn user_text(t: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            parts: vec![Part::Text { text: t.into() }],
        }
    }

    pub fn assistant_text(t: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            parts: vec![Part::Text { text: t.into() }],
        }
    }

    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    pub fn tool_calls(&self) -> Vec<ToolCall> {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::ToolCall { id, name, args, .. } => Some(ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    args: args.clone(),
                }),
                _ => None,
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments (object).
    pub parameters: serde_json::Value,
}

/// One block of the system prompt. `cache` marks the end of a stable prefix (prompt caching breakpoint).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemBlock {
    pub text: String,
    #[serde(default)]
    pub cache: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    /// Stable blocks first (rules, SOUL, ROLE, skills index), dynamic last (decision 36 work order).
    pub system: Vec<SystemBlock>,
    pub messages: Vec<Message>,
    /// Sorted by name by the caller, byte-stable (prompt cache).
    pub tools: Vec<ToolSpec>,
    pub effort: Effort,
    pub max_tokens: u32,
    /// Routing hint for prefix caches (OpenAI `prompt_cache_key`): `agent_id:ticket`.
    #[serde(default)]
    pub cache_key: Option<String>,
    /// Long-running agent: use the 1 hour cache TTL where the provider offers one.
    #[serde(default)]
    pub long_cache: bool,
    /// The tools are listed but may not be called (`tool_choice: none`): a notes request keeps the run's exact prefix
    /// (system, tools, history) so the provider's prompt cache serves it (ADR-113).
    #[serde(default)]
    pub tools_off: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    /// Safety classifier declined (Anthropic `refusal`, Gemini SAFETY, OpenAI content_filter).
    Refusal,
    /// The provider paused a long turn; send the conversation again to continue.
    PauseTurn,
    Other(String),
}

/// Token usage normalized across vendors (decision 3k: display only).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Uncached input tokens.
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// Of `cache_write`, the tokens written for an hour (Anthropic bills them 2x input, 5-minute writes 1.25x).
    #[serde(default)]
    pub cache_write_1h: u64,
    /// Reasoning tokens when reported separately (part of `output`).
    pub reasoning: u64,
}

impl Usage {
    pub fn add(&mut self, o: &Usage) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
        self.cache_write_1h += o.cache_write_1h;
        self.reasoning += o.reasoning;
    }

    pub fn total_input(&self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatResponse {
    pub message: Message,
    pub stop: StopReason,
    pub usage: Usage,
    pub model: String,
    /// Replayed thinking blocks the provider dropped, or flagged, because the history before them changed (Claude's
    /// preserved thinking; ADR-113). 0 where the provider has no such check.
    #[serde(default)]
    pub dropped_thinking: u32,
    /// Why the model declined ([`StopReason::Refusal`]): the vendor's category and explanation when it gives them
    /// (Anthropic's `stop_details`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<Refusal>,
}

/// A vendor's reason for a refusal; both parts may be missing (Anthropic: a refusal outside its named categories).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    /// `cyber`, `bio`, `frontier_llm`, `reasoning_extraction`, `general_harms` on Anthropic.
    pub category: Option<String>,
    /// Shown as it is (not stable text, never parsed).
    pub explanation: Option<String>,
}

/// Streaming notifications for live views. The final [`ChatResponse`] is authoritative.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallStarted {
        id: String,
        name: String,
    },
    ToolCall(ToolCall),
    Usage(Usage),
    /// A dropped stream is being retried: discard partial output shown so far.
    Restart,
    Done,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub max_output: Option<u64>,
    #[serde(default)]
    pub supports_tools: Option<bool>,
    #[serde(default)]
    pub supports_vision: Option<bool>,
    /// The model's effort levels, lowest first, by the vendor's names (reported, or known per model: `effort`);
    /// `None` = unknown, empty = the model has no effort control.
    #[serde(default)]
    pub efforts: Option<Vec<agenium_core::Effort>>,
    /// The level the vendor uses when none is sent.
    #[serde(default)]
    pub default_effort: Option<agenium_core::Effort>,
}
