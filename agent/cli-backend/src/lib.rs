//! CliBackend (decision 3c / ADR-30, milestone M2.5, ADR-56): run a vendor agent CLI headless (JSON-stream input
//! and output) as an Agenium agent. Agenium only *detects* installed CLIs — version and login state — and never logs
//! in. Each run gets an Agenium-owned per-run home with Agenium's configuration (its MCP server, its permission
//! rules), so the user's own CLI configuration (MCP servers, hooks, rules, skills) is neither used nor written; the
//! user's login is reused read-only.

pub mod agy;
pub mod detect;
pub mod driver;
pub mod run;

pub use detect::{detect_all, CliInfo, CliKind, Login};
pub use driver::{Driver, Event, Setup};
pub use run::{CliRun, Limits};

/// The driver of `kind`; `None` when Agenium does not drive that CLI (yet). Only CLIs whose headless contract and
/// per-run isolation were verified are listed (ADR-56).
pub fn driver(kind: CliKind) -> Option<&'static dyn Driver> {
    match kind {
        CliKind::Agy => Some(&agy::Agy),
        CliKind::Codex | CliKind::Devin | CliKind::Claude | CliKind::Opencode => None,
    }
}
