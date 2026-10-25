//! CliBackend (decision 3c / ADR-30, milestone M2.5, ADR-56): run a vendor agent CLI headless (JSON event stream)
//! in its own YOLO mode as an Agenium agent, with Agenium deciding the permissions (inside Agenium's sandbox, or
//! through the CLI's pre-tool hook). Agenium only *detects* installed CLIs — version and login state — and never
//! logs in. Each run gets an Agenium-owned per-run home with Agenium's configuration (its MCP server, its hooks), so
//! the user's own CLI configuration (MCP servers, hooks, rules, skills) is neither used nor modified.

pub mod agy;
pub mod claude;
pub mod codex;
pub mod detect;
pub mod driver;
pub mod opencode;
pub mod run;

pub use detect::{detect_all, CliInfo, CliKind, Login};
pub use driver::{Control, Driver, Event, Setup, Turns};
pub use run::{CliRun, Limits};

/// The driver of `kind`; `None` when Agenium does not drive that CLI (yet). Only CLIs whose headless contract and
/// per-run isolation were verified are listed (ADR-56).
pub fn driver(kind: CliKind) -> Option<&'static dyn Driver> {
    match kind {
        CliKind::Agy => Some(&agy::Agy),
        CliKind::Codex => Some(&codex::Codex),
        CliKind::Claude => Some(&claude::Claude),
        CliKind::Opencode => Some(&opencode::Opencode),
        CliKind::Devin => None,
    }
}
