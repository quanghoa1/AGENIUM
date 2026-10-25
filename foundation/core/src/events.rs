//! The append-only event log (event sourcing, decision 22) and the sink that feeds the realtime bus (ADR-45).

use serde::{Deserialize, Serialize};

use crate::agent::AgentStatus;
use crate::comment::CommentKind;
use crate::ids::{AgentId, CommentId, ProjectId, RunId, TeamId, TicketId, TicketKey};
use crate::ticket::TicketStatus;

/// Typed domain events. Serialized as `{ "kind": <variant>, "payload": {...} }`; the store keeps `kind` and the
/// payload separately so old logs stay readable when variants are added.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload")]
pub enum DomainEvent {
    ProjectCreated {
        project: ProjectId,
        name: String,
        path: String,
    },
    ProjectStatusChanged {
        project: ProjectId,
        status: String,
    },
    TeamCreated {
        team: TeamId,
        name: String,
        solo: bool,
    },
    /// `team` is `None` for an agent created into the pool (ADR-61).
    AgentCreated {
        agent: AgentId,
        name: String,
        #[serde(default)]
        team: Option<TeamId>,
        role: String,
    },
    /// The agent joined a team (`Some`) or went back to the pool (`None`).
    AgentTeamChanged {
        agent: AgentId,
        team: Option<TeamId>,
    },
    /// The team was pointed at a project (`Some`) or freed (`None`).
    TeamProjectChanged {
        team: TeamId,
        project: Option<ProjectId>,
    },
    /// A team was dissolved: its members went back to the pool.
    TeamDissolved {
        team: TeamId,
        name: String,
    },
    AgentUpdated {
        agent: AgentId,
        field: String,
    },
    /// The agent was deleted (decision 32); its row stays as a tombstone.
    AgentDeleted {
        agent: AgentId,
        name: String,
    },
    AgentStatusChanged {
        agent: AgentId,
        from: AgentStatus,
        to: AgentStatus,
    },
    TicketCreated {
        ticket: TicketId,
        key: TicketKey,
        title: String,
    },
    TicketAssigned {
        ticket: TicketId,
        key: TicketKey,
        agent: AgentId,
    },
    TicketStatusChanged {
        ticket: TicketId,
        key: TicketKey,
        from: TicketStatus,
        to: TicketStatus,
    },
    TicketApproved {
        ticket: TicketId,
        key: TicketKey,
    },
    TicketEscalated {
        ticket: TicketId,
        key: TicketKey,
        reason: String,
    },
    TicketUpdated {
        ticket: TicketId,
        key: TicketKey,
        fields: Vec<String>,
    },
    CommentAdded {
        comment: CommentId,
        ticket: TicketId,
        key: TicketKey,
        kind: CommentKind,
    },
    CommentResolved {
        comment: CommentId,
        ticket: TicketId,
    },
    RunStarted {
        run: RunId,
        agent: AgentId,
        ticket: Option<TicketId>,
    },
    RunEnded {
        run: RunId,
        agent: AgentId,
        outcome: String,
    },
    /// VCS operations are journal-first: `VcsOp` before touching git, `VcsOpDone` after (ADR-44).
    VcsOp {
        op: String,
        ticket: TicketId,
        key: TicketKey,
        detail: serde_json::Value,
    },
    VcsOpDone {
        op: String,
        ticket: TicketId,
        key: TicketKey,
        detail: serde_json::Value,
    },
    TicketMerged {
        ticket: TicketId,
        key: TicketKey,
        sha: String,
    },
    MergeFailed {
        ticket: TicketId,
        key: TicketKey,
        reason: String,
        files: Vec<String>,
    },
    PermissionGranted {
        ticket: TicketId,
        request: CommentId,
        scope: String,
    },
    PermissionDenied {
        ticket: TicketId,
        request: CommentId,
    },
    SpecProposed {
        project: ProjectId,
        version: u32,
    },
    SpecApproved {
        project: ProjectId,
        version: u32,
        by: String,
    },
    PlanProposed {
        project: ProjectId,
        version: u32,
        needs_user: bool,
    },
    PlanApproved {
        project: ProjectId,
        version: u32,
        by: String,
    },
    /// The Lead proposed a role its team lacks (decision 3l); waits for the user.
    RoleProposed {
        project: ProjectId,
        proposal: String,
        role: String,
    },
    /// The user approved (an agent joined the team) or declined a role proposal.
    RoleProposalDecided {
        project: ProjectId,
        proposal: String,
        approved: bool,
        #[serde(default)]
        agent: Option<AgentId>,
    },
    /// The planner's draft of plan v1 (ADR-59); kept in the log, read by the Lead's inbox.
    PlanDrafted {
        project: ProjectId,
        spec_version: u32,
        by: AgentId,
        draft: crate::plan::PlanDraft,
    },
    PlanItemDone {
        project: ProjectId,
        item: String,
    },
    LinkAdded {
        ticket: TicketId,
        from: TicketKey,
        to: TicketKey,
        kind: String,
        confirmed: bool,
    },
    /// Something the user must look at ("Needs you"): approvals outside the allowlist, escalations, quota.
    NeedsUser {
        ticket: Option<TicketId>,
        reason: String,
    },
}

impl DomainEvent {
    /// Ticket the event belongs to (indexed for per-ticket timelines).
    pub fn ticket(&self) -> Option<TicketId> {
        use DomainEvent::*;
        match self {
            TicketCreated { ticket, .. }
            | TicketAssigned { ticket, .. }
            | TicketStatusChanged { ticket, .. }
            | TicketApproved { ticket, .. }
            | TicketEscalated { ticket, .. }
            | TicketUpdated { ticket, .. }
            | CommentAdded { ticket, .. }
            | CommentResolved { ticket, .. }
            | VcsOp { ticket, .. }
            | VcsOpDone { ticket, .. }
            | TicketMerged { ticket, .. }
            | MergeFailed { ticket, .. }
            | PermissionGranted { ticket, .. }
            | PermissionDenied { ticket, .. }
            | LinkAdded { ticket, .. } => Some(*ticket),
            RunStarted { ticket, .. } | NeedsUser { ticket, .. } => *ticket,
            _ => None,
        }
    }

    /// Splits into `(kind, payload)` for storage.
    pub fn into_parts(self) -> (String, serde_json::Value) {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(mut m)) => {
                let kind = m
                    .remove("kind")
                    .and_then(|k| k.as_str().map(str::to_owned))
                    .unwrap_or_default();
                let payload = m.remove("payload").unwrap_or(serde_json::Value::Null);
                (kind, payload)
            }
            // A derived Serialize on this enum always yields an object.
            _ => unreachable!("DomainEvent serializes to an object"),
        }
    }
}

/// An event to append. `intent_id` makes the append idempotent: a second append with the same id returns the
/// first record and changes nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct EventDraft {
    pub intent_id: Option<String>,
    pub actor: String,
    pub event: DomainEvent,
}

impl EventDraft {
    pub fn new(actor: impl Into<String>, event: DomainEvent) -> Self {
        Self {
            intent_id: None,
            actor: actor.into(),
            event,
        }
    }

    pub fn with_intent(mut self, intent: impl Into<String>) -> Self {
        self.intent_id = Some(intent.into());
        self
    }
}

/// A committed event. `seq` is gap-free and strictly increasing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    pub seq: u64,
    pub intent_id: Option<String>,
    pub kind: String,
    pub ticket: Option<TicketId>,
    pub actor: String,
    pub payload: serde_json::Value,
    pub at: String,
}

impl EventRecord {
    /// Decodes the typed event; `None` for kinds unknown to this build.
    pub fn decode(&self) -> Option<DomainEvent> {
        serde_json::from_value(serde_json::json!({ "kind": self.kind, "payload": self.payload }))
            .ok()
    }
}

/// Receives every newly committed event, after the transaction commits (never for idempotent replays).
/// Implemented by the realtime bus; must not block.
pub trait EventSink: Send + Sync {
    fn emit(&self, e: &EventRecord);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_round_trip() {
        let t = TicketId::new();
        let e = DomainEvent::TicketStatusChanged {
            ticket: t,
            key: "AGN-3".parse().unwrap(),
            from: TicketStatus::New,
            to: TicketStatus::Assigned,
        };
        assert_eq!(e.ticket(), Some(t));
        let (kind, payload) = e.clone().into_parts();
        assert_eq!(kind, "TicketStatusChanged");
        let rec = EventRecord {
            seq: 1,
            intent_id: None,
            kind,
            ticket: Some(t),
            actor: "user".into(),
            payload,
            at: String::new(),
        };
        assert_eq!(rec.decode(), Some(e));
        let unknown = EventRecord {
            kind: "FromTheFuture".into(),
            ..rec
        };
        assert_eq!(unknown.decode(), None);
    }
}
