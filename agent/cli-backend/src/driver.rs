//! What the CliBackend needs from one vendor CLI (ADR-56): how to start it headless with JSON-stream input and output,
//! how to point it at an Agenium-owned per-run home instead of the user's own configuration (ADR-30), and how to read
//! its event stream.

use std::collections::BTreeMap;
use std::path::Path;

use agenium_llm::Usage;
use serde_json::Value;

/// Everything a driver writes into the per-run home.
pub struct Setup<'a> {
    /// Fresh, empty directory owned by this run (deleted afterwards).
    pub home: &'a Path,
    /// Ticket worktree: the CLI's working directory and its only workspace.
    pub worktree: &'a Path,
    /// Agenium's MCP server for this run.
    pub mcp_name: &'a str,
    pub mcp_url: &'a str,
    pub mcp_authorization: &'a str,
    /// Exact shell commands the CLI may run without asking (the project allowlist). Everything else is refused.
    pub allowed_commands: &'a [String],
}

/// One thing the CLI reported.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The CLI's own conversation id (diagnostics only: Agenium never resumes CLI sessions).
    Session(String),
    /// Assistant text (a delta).
    Text(String),
    /// A tool step of the CLI; `done` = finished (output is not reported by every CLI).
    Tool {
        id: String,
        name: String,
        input: Value,
        done: bool,
    },
    /// The turn ended. `denied` lists the permission kinds the CLI refused itself (headless mode cannot ask).
    TurnEnd {
        usage: Usage,
        response: String,
        denied: Vec<String>,
        error: Option<String>,
    },
}

pub trait Driver: Send + Sync {
    /// Arguments for a headless session that reads user messages from stdin and writes JSON events to stdout.
    fn args(&self, model: &str) -> Vec<String>;
    /// Writes the per-run configuration into `setup.home` and returns the environment that makes the CLI use it
    /// instead of the user's own configuration.
    fn prepare(&self, setup: &Setup) -> std::io::Result<BTreeMap<String, String>>;
    /// One stdin line carrying a user message.
    fn user_line(&self, text: &str) -> String;
    /// Events of one stdout line.
    fn parse(&self, line: &Value) -> Vec<Event>;
    /// The shell command a tool step runs, when it runs one.
    fn command(&self, tool: &str, input: &Value) -> Option<String>;
    /// Whether a tool step is the CLI calling a tool of Agenium's own MCP server (Agenium logs those itself).
    fn is_agenium_call(&self, tool: &str, input: &Value, server: &str) -> bool;
}

/// Whether `command` is allowed by the project allowlist (exact entries; `prefix *` entries match the prefix when
/// the command has no shell control characters — the same rule as the API backend's command gate).
pub fn permitted(command: &str, allowed: &[String]) -> bool {
    let c = command.trim();
    let plain = !c.contains([';', '&', '|', '`', '$', '>', '<', '\n', '(', ')']);
    allowed.iter().any(|e| match e.strip_suffix('*') {
        Some(prefix) => plain && c.starts_with(prefix),
        None => c == e.trim(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_rules() {
        let allowed = vec!["cargo test".to_owned(), "npm run *".to_owned()];
        assert!(permitted("cargo test", &allowed));
        assert!(!permitted("cargo test; curl x", &allowed));
        assert!(permitted("npm run build", &allowed));
        assert!(!permitted("npm run build && curl x", &allowed));
        assert!(!permitted("git status", &allowed));
    }
}
