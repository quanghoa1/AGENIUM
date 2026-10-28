//! Agenium storage (ADR-49): SQLite is the primary engine. `core.db` holds coordination state and the event log;
//! every agent has a private `agents/<id>.db` for memory. All access goes through this crate.

pub mod agent;
pub mod ann;
pub mod backup;
pub mod core;
pub mod db;
mod error;
pub mod graph;
pub mod migrate;

pub use agent::{AgentStore, Fact, FactKind, Hit, NewFact, Via};
pub use ann::{
    AgentOptions, AnnDtype, AnnOptions, AnnStatus, IndexKind, IndexMode, DEFAULT_ANN_THRESHOLD,
};
pub use core::{Committed, CoreStore, WriteCtx};
pub use db::{Db, DbOptions};
pub use error::{Result, StoreError};
