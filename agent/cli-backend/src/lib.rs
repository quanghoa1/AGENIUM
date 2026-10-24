//! CliBackend (decision 3c / ADR-30, milestone M2.5, ADR-56): run a vendor agent CLI as an Agenium agent. Agenium
//! only *detects* installed CLIs — version and login state — and never logs in, never writes the user's global CLI
//! configuration and reuses the user's own authentication read-only.
//!
//! Runs use the Agent Client Protocol (`acp`): the CLI is an ACP server on stdio, Agenium is the client and offers
//! its protocol tools through its own MCP server (`agenium_mcp::server`) given to the session (`run`).

pub mod acp;
pub mod detect;
pub mod run;

pub use detect::{detect_all, CliInfo, CliKind, Login};
pub use run::{CliRun, Limits};

/// Arguments that start `kind` as an ACP server with `model` (empty = the CLI's default); `None` when Agenium does
/// not drive that CLI over ACP. Only verified CLIs are listed (ADR-56).
pub fn acp_args(kind: CliKind, model: &str) -> Option<Vec<String>> {
    match kind {
        CliKind::Devin => {
            let mut a = vec!["acp".to_owned()];
            if !model.is_empty() {
                a.extend(["--model".to_owned(), model.to_owned()]);
            }
            Some(a)
        }
        CliKind::Codex | CliKind::Agy | CliKind::Claude | CliKind::Opencode => None,
    }
}
