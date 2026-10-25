//! Orchestrator: the only component that applies rules and writes state (decision 36). Agents propose actions
//! through typed intents; the engine validates them against the database and commits deterministically.

pub mod agent_io;
pub mod app;
pub mod effects;
pub mod engine;
mod error;
pub mod inbox;
pub mod maintenance;
pub mod manifest;
pub mod mcp;
pub mod prices;
pub mod registry;
pub mod role_library;
pub mod role_proposals;
pub mod runtime;
pub mod sched;
pub mod team_templates;
pub mod templates;
pub mod vendor_usage;
pub mod views;
pub mod worktree_guard;

pub use app::{App, AppOptions, CheckFactory, CreateAgent, CreateProject, NoCheckFactory};
pub use engine::{CallCtx, Outcome, RunEffect};
pub use error::{OrchError, Result, ToolError};
