//! `[tool_search]`: an agent finds its MCP tools instead of getting them all listed (ADR-139).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolSearchSection {
    /// `auto`: the MCP tools are found with `tool_search` and called with `call_tool` once an agent has more than
    /// `over_tools` of them or their definitions take more than `over_tokens`; `always`; `never` (every tool listed).
    pub mode: String,
    pub over_tools: usize,
    pub over_tokens: usize,
    /// The MCP tools an agent used most that stay listed when the others are found by search.
    pub keep_used: usize,
    /// Let the judge (Jev) rate the best candidates of a search; fail-open (without it, words and meaning rank them).
    pub judge: bool,
}

impl Default for ToolSearchSection {
    fn default() -> Self {
        Self {
            mode: "auto".into(),
            over_tools: 20,
            over_tokens: 10_000,
            keep_used: 5,
            judge: true,
        }
    }
}

impl ToolSearchSection {
    /// Whether tools of this count and size are found by search rather than listed.
    pub fn defers(&self, tools: usize, tokens: usize) -> bool {
        match self.mode.as_str() {
            "always" => tools > 0,
            "never" => false,
            _ => tools > self.over_tools || tokens > self.over_tokens,
        }
    }
}
