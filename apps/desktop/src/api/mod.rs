//! IPC commands of the desktop app. Read models come from the orchestrator (`views`, `inbox`, store queries); every
//! action goes through the same App/Runtime methods as the CLI, so the rules are the orchestrator's, never the UI's.

use std::collections::{BTreeMap, HashMap};

use agenium_core::forms::{self, PermissionDecision};
use agenium_core::*;
use agenium_orchestrator::app::{CreateAgent, CreateProject};
use agenium_orchestrator::runtime::Approval;
use agenium_orchestrator::CallCtx;
use agenium_store::core::{comments, events, runs, tickets, workspaces};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, State};

use crate::state::{err, Desktop, Opened, R};

mod actions;
mod agents;
mod cli_updates;
mod design;
mod markitdown;
mod overview;
mod skill_quality;
mod status;
mod ticket_view;

pub use actions::*;
pub use agents::*;
pub use cli_updates::*;
pub use design::*;
pub use markitdown::*;
pub use overview::*;
pub use skill_quality::*;
pub use status::*;
pub use ticket_view::*;

pub(crate) fn key(s: &str) -> R<TicketKey> {
    s.parse().map_err(|e: IdParseError| e.to_string())
}

pub(crate) fn pid(s: &str) -> R<ProjectId> {
    s.parse().map_err(|e: IdParseError| e.to_string())
}

pub(crate) fn names(o: &Opened) -> R<HashMap<AgentId, String>> {
    Ok(o.app
        .agents()
        .map_err(err)?
        .into_iter()
        .map(|a| (a.id, a.name))
        .collect())
}
