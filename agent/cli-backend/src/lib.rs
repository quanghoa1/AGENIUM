//! CliBackend (decision 3c / ADR-30, milestone M2.5, ADR-56): run a vendor agent CLI headless (JSON event stream)
//! in its own YOLO mode as an Agenium agent, with Agenium deciding the permissions (inside Agenium's sandbox, or
//! through the CLI's pre-tool hook). Agenium only *detects* installed CLIs — version and login state — and never
//! logs in. Each run gets an Agenium-owned per-run home with Agenium's configuration (its MCP server, its hooks), so
//! the user's own CLI configuration (MCP servers, hooks, rules, skills) is neither used nor modified.

pub mod agy;
pub mod claude;
pub mod codex;
pub mod detect;
pub mod devin;
pub mod driver;
pub mod grok;
pub mod login;
pub mod login_access;
pub mod opencode;
pub mod run;
mod tool_paths;
pub mod updates;

pub use detect::{detect_all, CliInfo, CliKind, Login};
pub use driver::{Control, Driver, Event, Setup, Turns};
pub use run::{CliRun, Limits};

/// How long one of Agenium's tools may keep a CLI waiting (ADR-133): a `shell` command still going after it goes on as
/// a job, its end handed to the agent as a message at its turn's end. Under every CLI's own wait for a tool (agy and
/// OpenCode have no setting for theirs; they waited 80 s).
pub const FOREGROUND_LIMIT: std::time::Duration = std::time::Duration::from_secs(45);

/// The driver of `kind`; `None` when Agenium does not drive that CLI (yet). Only CLIs whose headless contract and
/// per-run isolation were verified are listed (ADR-56).
pub fn driver(kind: CliKind) -> Option<&'static dyn Driver> {
    match kind {
        CliKind::Agy => Some(&agy::Agy),
        CliKind::Codex => Some(&codex::Codex),
        CliKind::Claude => Some(&claude::Claude),
        CliKind::Opencode => Some(&opencode::Opencode),
        CliKind::Grok => Some(&grok::Grok),
        CliKind::Devin => Some(&devin::Devin),
    }
}
