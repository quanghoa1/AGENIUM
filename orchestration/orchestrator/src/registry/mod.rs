//! Agent registry (ADR-61): agents are long-lived and created on their own into the pool (architecture page
//! "Agent registry": the user creates agents first), each with a role, its own model, the library skills and MCP
//! servers it enables; teams are then composed from pool agents (one Lead, workers, at most one planner). An agent
//! is in at most one team at a time (decision 3). The system only suggests a Lead; the user decides.
//!
//! Also here: roles (template presets and the user's own), the system skill library and model providers.

use std::path::Path;

use agenium_config::{ProviderConfig, ProviderKind};
use agenium_core::*;
use agenium_store::core::agents;
use agenium_store::StoreError;
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::team_templates::RoleFile;
use crate::{OrchError, Result};

mod library;
mod pool;
mod providers;
mod roles;
mod teams;

pub use pool::*;
pub use providers::*;
pub use roles::*;
pub use teams::*;

fn invalid(msg: impl Into<String>) -> OrchError {
    OrchError::Invalid(msg.into())
}

fn io(e: impl std::fmt::Display) -> OrchError {
    OrchError::Invalid(e.to_string())
}

/// Longest role id.
pub(crate) const ROLE_ID_MAX: usize = 60;

fn slug_ok(s: &str) -> bool {
    agenium_core::validate::slug_ok(s, ROLE_ID_MAX)
}

impl App {
    // ------------------------------------------------------------ roles
}
