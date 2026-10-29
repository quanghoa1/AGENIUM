//! When an agent's MCP tool call waits for the user (ADR-143): `[sandbox] mcp_tools` for every server, `approval` on
//! one `[[mcp]]` server to differ from it.

use serde::{Deserialize, Serialize};

/// Which MCP tool calls go to the user before they run. The server says what a tool does in its annotations
/// (`readOnlyHint`, `destructiveHint`); a call the user allowed always is on the project's allowlist (`mcp:<server>/<tool>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum McpApproval {
    /// The tools the server marks destructive (`destructiveHint: true`: deleting, refunding, sending).
    #[default]
    Destructive,
    /// Every tool the server does not mark read-only (a server that says nothing of its tools).
    Writes,
    /// None: every tool runs without asking.
    Never,
}

impl McpApproval {
    /// Whether a call of a tool with these annotations waits for the user.
    pub fn asks(self, read_only: bool, destructive: Option<bool>) -> bool {
        match self {
            McpApproval::Never => false,
            McpApproval::Writes => !read_only,
            McpApproval::Destructive => !read_only && destructive == Some(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_waits_for_the_user() {
        use McpApproval::*;
        assert!(Destructive.asks(false, Some(true)));
        assert!(!Destructive.asks(false, None));
        assert!(!Destructive.asks(true, Some(true)));
        assert!(Writes.asks(false, None));
        assert!(!Writes.asks(true, None));
        assert!(!Never.asks(false, Some(true)));
    }
}
