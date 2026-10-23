//! `<root>/config.toml`. Every section has defaults so a partial (or older) file loads; unknown keys are ignored
//! so a newer file still loads in an older build.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ConfigError;

pub const CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub schema_version: u32,
    pub app: AppSection,
    pub paths: PathsSection,
    pub runtime: RuntimeSection,
    pub retry: RetrySection,
    pub usage: UsageSection,
    pub backup: BackupSection,
    pub memory: MemorySection,
    pub secrets: SecretsSection,
    pub sandbox: SandboxSection,
    pub providers: Vec<ProviderConfig>,
    /// MCP servers (`[[mcp]]`); agents use them through their bindings.
    pub mcp: Vec<McpServerConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            app: AppSection::default(),
            paths: PathsSection::default(),
            runtime: RuntimeSection::default(),
            retry: RetrySection::default(),
            usage: UsageSection::default(),
            backup: BackupSection::default(),
            memory: MemorySection::default(),
            secrets: SecretsSection::default(),
            sandbox: SandboxSection::default(),
            providers: Vec::new(),
            mcp: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSection {
    /// UI language: "en" | "vi".
    pub language: String,
    /// Language the user talks to the Lead/Solo in (free-form BCP-47). Everything below the Lead is English.
    pub user_language: String,
    pub theme: String,
    /// Decision 26: off, never sends anything.
    pub telemetry: bool,
    pub crash_report: bool,
}

impl Default for AppSection {
    fn default() -> Self {
        Self {
            language: "en".into(),
            user_language: "en".into(),
            theme: "system".into(),
            telemetry: false,
            crash_report: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PathsSection {
    /// Relative to the root, or absolute.
    pub projects_dir: String,
    pub backups_dir: String,
}

impl Default for PathsSection {
    fn default() -> Self {
        Self {
            projects_dir: "projects".into(),
            backups_dir: "backups".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeSection {
    /// Lowered automatically on machines below the recommended spec.
    pub max_parallel_agents: u32,
    pub max_steps_per_run: u32,
    /// Same tool with identical arguments this many times in a row stops the run (decision 17).
    pub loop_detect_same_call: u32,
    /// Consecutive invalid tool calls that stop the run (decision 36).
    pub tool_error_stop: u32,
    pub stall_minutes: u32,
    pub stall_nudges: u32,
    pub blocking_question_sla_minutes: u32,
    pub max_reopen_before_escalate: u32,
    pub shell_timeout_secs: u32,
    pub shell_timeout_max_secs: u32,
    /// Tool output returned to the model is cut to this many characters / lines (full output kept as artifact).
    pub tool_output_max_chars: u32,
    pub tool_output_max_lines: u32,
}

impl Default for RuntimeSection {
    fn default() -> Self {
        Self {
            max_parallel_agents: 4,
            max_steps_per_run: 200,
            loop_detect_same_call: 3,
            tool_error_stop: 3,
            stall_minutes: 5,
            stall_nudges: 3,
            blocking_question_sla_minutes: 10,
            max_reopen_before_escalate: 3,
            shell_timeout_secs: 120,
            shell_timeout_max_secs: 600,
            tool_output_max_chars: 30_000,
            tool_output_max_lines: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetrySection {
    pub base_ms: u64,
    pub max_backoff_ms: u64,
    pub max_attempts: u32,
    pub max_total_secs: u64,
}

impl Default for RetrySection {
    fn default() -> Self {
        Self {
            base_ms: 1000,
            max_backoff_ms: 30_000,
            max_attempts: 8,
            max_total_secs: 300,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageSection {
    /// Display only (decision 3k): nothing is ever blocked by cost or tokens.
    pub show_tokens: bool,
    pub show_estimated_cost: bool,
    pub pricing_file: String,
}

impl Default for UsageSection {
    fn default() -> Self {
        Self {
            show_tokens: true,
            show_estimated_cost: true,
            pricing_file: "pricing.toml".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupSection {
    pub hourly_keep: u32,
    pub daily_keep: u32,
    pub weekly_keep: u32,
    pub pre_migrate_keep: u32,
}

impl Default for BackupSection {
    fn default() -> Self {
        Self {
            hourly_keep: 24,
            daily_keep: 7,
            weekly_keep: 4,
            pre_migrate_keep: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemorySection {
    /// `<kind>:<model>`; memory is English-only (ADR-40), so an English model is the default.
    pub embedder: String,
    pub language: String,
    /// Facts per agent from which the ANN sidecar replaces exact search (ADR-50).
    pub ann_threshold: u64,
    /// Facts injected into a work order.
    pub top_k: u32,
}

impl Default for MemorySection {
    fn default() -> Self {
        Self {
            embedder: "local:bge-small-en-v1.5".into(),
            language: "en".into(),
            ann_threshold: 100_000,
            top_k: 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SecretsBackend {
    /// Encrypted vault in the root; only its master key is in the OS keyring (spike S5 decision).
    #[default]
    Vault,
    /// Every secret directly in the OS keyring.
    Keyring,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SecretsSection {
    pub backend: SecretsBackend,
}

/// Approval policy for commands/writes that the sandbox does not cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalPolicy {
    /// Before a sandbox backend is verified on this OS: every shell command and every write outside the worktree
    /// needs the user's approval (M1-7).
    #[default]
    OnRequest,
    /// YOLO: run without asking, inside the sandbox only. Refused when no sandbox backend works (fail-closed).
    Never,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SandboxSection {
    pub approval: ApprovalPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Anthropic,
    Gemini,
    /// OpenAI, OpenRouter, Opencode, DeepSeek and Local/custom endpoints.
    OpenaiCompat,
    /// Deterministic scripted provider used by tests and evals (no network).
    Mock,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub kind: ProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Name of the secret holding the API key (e.g. `provider/anthropic`); empty = no key (local endpoints).
    #[serde(default)]
    pub api_key_ref: String,
    /// Concurrent requests allowed per API key.
    #[serde(default = "default_concurrency")]
    pub max_concurrency: u32,
    /// Local/custom endpoints: models listed manually when the endpoint cannot list them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
}

fn default_concurrency() -> u32 {
    4
}

/// An MCP server: either a local process (`command`, stdio) or a remote endpoint (`url`, Streamable HTTP).
/// Values in `env` and `headers` are plain text or contain placeholders, resolved when connecting:
/// `${secret:<name>}` (a vault secret) or `${cred:<KEY>}` (the secret the agent's binding maps `KEY` to, so two
/// agents can use different keys for the same server). Secrets never appear in this file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub env: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub headers: std::collections::BTreeMap<String, String>,
    /// Seconds a tool call may take.
    #[serde(default = "default_mcp_timeout")]
    pub timeout_secs: u64,
}

fn default_mcp_timeout() -> u64 {
    120
}

/// MCP server ids: 1-32 characters of `a-z`, `0-9`, `-` (they become part of tool names).
pub fn valid_mcp_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
}

fn is_local_http(u: &str) -> bool {
    ["http://localhost", "http://127.0.0.1", "http://[::1]"]
        .iter()
        .any(|p| {
            u.strip_prefix(p)
                .is_some_and(|r| r.is_empty() || r.starts_with([':', '/']))
        })
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = fs::read_to_string(path).map_err(|e| ConfigError::io(path, e))?;
        Self::parse(&text).map_err(|message| ConfigError::Parse {
            path: path.into(),
            message,
        })
    }

    pub fn parse(text: &str) -> Result<Config, String> {
        let c: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        c.check()?;
        Ok(c)
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let text = toml::to_string_pretty(self).map_err(|e| ConfigError::Parse {
            path: path.into(),
            message: e.to_string(),
        })?;
        crate::write_atomic(path, text.as_bytes())
    }

    /// Semantic checks beyond types.
    pub fn check(&self) -> Result<(), String> {
        if self.schema_version > CONFIG_SCHEMA_VERSION {
            return Err(format!(
                "config schema_version {} is newer than this build ({CONFIG_SCHEMA_VERSION})",
                self.schema_version
            ));
        }
        if !matches!(self.app.language.as_str(), "en" | "vi") {
            return Err(format!(
                "app.language must be \"en\" or \"vi\", got {:?}",
                self.app.language
            ));
        }
        if self.runtime.max_steps_per_run == 0 || self.runtime.max_parallel_agents == 0 {
            return Err(
                "runtime.max_steps_per_run and runtime.max_parallel_agents must be > 0".into(),
            );
        }
        let mut ids = std::collections::HashSet::new();
        for p in &self.providers {
            if p.id.trim().is_empty() || !ids.insert(p.id.as_str()) {
                return Err(format!("provider id {:?} is empty or duplicated", p.id));
            }
            if p.kind == ProviderKind::OpenaiCompat
                && p.base_url.as_deref().unwrap_or_default().is_empty()
            {
                return Err(format!("provider {:?}: openai_compat needs base_url", p.id));
            }
        }
        let mut mcp_ids = std::collections::HashSet::new();
        for m in &self.mcp {
            if !valid_mcp_id(&m.id) || !mcp_ids.insert(m.id.as_str()) {
                return Err(format!(
                    "mcp server id {:?} must be unique and use a-z, 0-9, - (max 32)",
                    m.id
                ));
            }
            match (&m.command, &m.url) {
                (Some(c), None) if !c.trim().is_empty() => {}
                (None, Some(u)) if u.starts_with("https://") || is_local_http(u) => {}
                (None, Some(u)) => {
                    return Err(format!(
                        "mcp server {:?}: url must be https (http only for localhost), got {u:?}",
                        m.id
                    ))
                }
                _ => {
                    return Err(format!(
                        "mcp server {:?} needs exactly one of command (stdio) or url (HTTP)",
                        m.id
                    ))
                }
            }
        }
        Ok(())
    }

    pub fn mcp_server(&self, id: &str) -> Option<&McpServerConfig> {
        self.mcp.iter().find(|m| m.id == id)
    }

    pub fn provider(&self, id: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
schema_version = 1
[app]
language = "vi"
user_language = "vi"
[runtime]
max_parallel_agents = 2
[[providers]]
id = "anthropic"
kind = "anthropic"
api_key_ref = "provider/anthropic"
[[providers]]
id = "local-ollama"
kind = "openai_compat"
base_url = "http://localhost:11434/v1"
some_future_key = 1
"#;

    #[test]
    fn partial_file_gets_defaults() {
        let c = Config::parse(EXAMPLE).unwrap();
        assert_eq!(c.app.user_language, "vi");
        assert_eq!(c.runtime.max_parallel_agents, 2);
        assert_eq!(c.runtime.max_steps_per_run, 200);
        assert_eq!(c.retry, RetrySection::default());
        assert_eq!(c.providers[1].max_concurrency, 4);
        assert_eq!(c.secrets.backend, SecretsBackend::Vault);
        assert_eq!(c.sandbox.approval, ApprovalPolicy::OnRequest);
        assert!(!c.app.telemetry);
    }

    #[test]
    fn round_trip_and_checks() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("config.toml");
        let c = Config::parse(EXAMPLE).unwrap();
        c.save(&p).unwrap();
        assert_eq!(Config::load(&p).unwrap(), c);
        assert!(Config::parse("[app]\nlanguage = \"fr\"").is_err());
        assert!(Config::parse("schema_version = 99").is_err());
        assert!(Config::parse("[[providers]]\nid=\"x\"\nkind=\"openai_compat\"").is_err());
        assert!(Config::parse(
            "[[providers]]\nid=\"x\"\nkind=\"mock\"\n[[providers]]\nid=\"x\"\nkind=\"mock\""
        )
        .is_err());
    }
}
