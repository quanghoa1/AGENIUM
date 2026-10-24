//! Agenium domain core: identifiers, tickets, agents, typed comments and forms, the protocol tool catalogue,
//! the ticket state machine and the dependency rule. Pure (no I/O) so everything here is unit/property tested.

pub mod agent;
pub mod comment;
pub mod events;
pub mod forms;
pub mod graph;
pub mod ids;
pub mod language;
pub mod plan;
pub mod protocol;
pub mod rules;
pub mod ticket;
pub mod validate;

pub use agent::{AgentStatus, BackendKind, Effort, ModelBinding, RoleKind};
pub use comment::{Author, Comment, CommentBody, CommentKind};
pub use events::{DomainEvent, EventDraft, EventRecord, EventSink};
pub use ids::{
    AgentId, CommentId, IdParseError, IntentId, ProjectId, RunId, TeamId, TicketId, TicketKey,
};
pub use protocol::{Intent, IntentError, ProtocolTool};
pub use rules::{apply_transition, check_transition, dep_satisfied, Actor, Guards, RuleError};
pub use ticket::{AcceptanceCriterion, Ticket, TicketKind, TicketStatus, UsageSummary};

/// Current UTC time in the canonical storage format (`2026-10-23T05:12:00.123Z`), identical to SQLite's
/// `strftime('%Y-%m-%dT%H:%M:%fZ','now')` so text comparison orders correctly.
pub fn now_iso() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}
