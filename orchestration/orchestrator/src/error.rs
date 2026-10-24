use agenium_core::protocol::IntentError;
use agenium_core::validate::FormErrors;
use agenium_core::RuleError;
use agenium_store::StoreError;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum OrchError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Rule(#[from] RuleError),
    #[error(transparent)]
    Intent(#[from] IntentError),
    #[error("invalid form: {0}")]
    Form(#[from] FormErrors),
    #[error(transparent)]
    Vcs(#[from] agenium_vcs::VcsError),
    #[error(transparent)]
    Config(#[from] agenium_config::ConfigError),
    #[error(transparent)]
    Secret(#[from] agenium_secrets::SecretError),
    #[error("not found: {0}")]
    NotFound(String),
    /// The actor may not do this (role, ownership, scope).
    #[error("denied: {0}")]
    Denied(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid: {0}")]
    Invalid(String),
}

/// Structured error returned to a model as a tool result (decision 36: code + reason + current state).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl OrchError {
    pub fn code(&self) -> &'static str {
        match self {
            OrchError::Store(StoreError::NotFound(_)) | OrchError::NotFound(_) => "not_found",
            OrchError::Store(StoreError::Conflict(_)) | OrchError::Conflict(_) => "conflict",
            OrchError::Store(_) => "storage_error",
            OrchError::Rule(RuleError::Terminal { .. }) => "terminal_state",
            OrchError::Rule(RuleError::Forbidden { .. }) => "forbidden_transition",
            OrchError::Rule(RuleError::Guard { .. }) => "guard_failed",
            OrchError::Intent(IntentError::UnknownTool { .. }) => "unknown_tool",
            OrchError::Intent(IntentError::NotAllowed { .. }) => "tool_not_allowed",
            OrchError::Intent(_) | OrchError::Form(_) => "invalid_arguments",
            OrchError::Vcs(agenium_vcs::VcsError::SecretsFound(_)) => "secret_detected",
            OrchError::Vcs(_) => "git_error",
            OrchError::Config(_) => "config_error",
            OrchError::Secret(_) => "secret_store_error",
            OrchError::Denied(_) => "denied",
            OrchError::Invalid(_) => "invalid",
        }
    }

    pub fn to_tool_error(&self) -> ToolError {
        let detail = match self {
            OrchError::Form(f) | OrchError::Intent(IntentError::Fields { errors: f, .. }) => {
                serde_json::to_value(f).ok()
            }
            OrchError::Rule(r) => serde_json::to_value(r).ok(),
            OrchError::Vcs(agenium_vcs::VcsError::SecretsFound(f)) => serde_json::to_value(f).ok(),
            _ => None,
        };
        ToolError {
            code: self.code().to_owned(),
            message: self.to_string(),
            detail,
        }
    }
}

pub type Result<T, E = OrchError> = std::result::Result<T, E>;
