//! Tickets: the single source of truth for coordination (decision 3d).

use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::{AgentId, ProjectId, TicketId, TicketKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum TicketKind {
    Epic,
    Story,
    Task,
    Bug,
    /// Discovery ticket that produces the RequirementSpec.
    Requirement,
}

/// Ticket lifecycle (decision 3i). There is deliberately no `Merging` state: merge outcome is the `merged` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum TicketStatus {
    New,
    Assigned,
    InProgress,
    Pending,
    Resolved,
    Reopened,
    Closed,
    Cancelled,
}

impl TicketStatus {
    pub const ALL: [TicketStatus; 8] = [
        Self::New,
        Self::Assigned,
        Self::InProgress,
        Self::Pending,
        Self::Resolved,
        Self::Reopened,
        Self::Closed,
        Self::Cancelled,
    ];

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Closed | Self::Cancelled)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Assigned => "Assigned",
            Self::InProgress => "InProgress",
            Self::Pending => "Pending",
            Self::Resolved => "Resolved",
            Self::Reopened => "Reopened",
            Self::Closed => "Closed",
            Self::Cancelled => "Cancelled",
        }
    }
}

impl fmt::Display for TicketStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TicketStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| format!("unknown ticket status {s:?}"))
    }
}

impl TicketKind {
    pub const ALL: [TicketKind; 5] = [
        Self::Epic,
        Self::Story,
        Self::Task,
        Self::Bug,
        Self::Requirement,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Epic => "Epic",
            Self::Story => "Story",
            Self::Task => "Task",
            Self::Bug => "Bug",
            Self::Requirement => "Requirement",
        }
    }
}

impl FromStr for TicketKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| format!("unknown ticket kind {s:?}"))
    }
}

/// One acceptance criterion; ids look like `AC-1` and are referenced by reports and reviews.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceCriterion {
    #[schemars(pattern(r"^AC-[0-9]+$"))]
    pub id: String,
    #[schemars(length(min = 1))]
    pub text: String,
}

/// Tokens and estimated cost. Display only: never blocks anything (decision 3k).
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UsageSummary {
    #[serde(default)]
    pub tokens_in: u64,
    #[serde(default)]
    pub tokens_out: u64,
    #[serde(default)]
    pub cache_read: u64,
    #[serde(default)]
    pub cache_write: u64,
    #[serde(default)]
    pub estimated_cost_usd: f64,
    #[serde(default)]
    pub elapsed_ms: u64,
}

impl UsageSummary {
    pub fn add(&mut self, o: &UsageSummary) {
        self.tokens_in += o.tokens_in;
        self.tokens_out += o.tokens_out;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
        self.estimated_cost_usd += o.estimated_cost_usd;
        self.elapsed_ms += o.elapsed_ms;
    }
}

/// A ticket as stored and as shown to agents (work order item 7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ticket {
    pub id: TicketId,
    pub key: TicketKey,
    pub project: ProjectId,
    pub parent: Option<TicketKey>,
    pub kind: TicketKind,
    pub title: String,
    pub description: String,
    pub acceptance: Vec<AcceptanceCriterion>,
    pub status: TicketStatus,
    pub assignee: Option<AgentId>,
    /// Mandatory: every ticket traces to a plan item (decision 3h).
    pub plan_item: String,
    /// 0 (P0, highest) ..= 3.
    pub priority: u8,
    pub depends_on: Vec<TicketKey>,
    pub reopen_count: u32,
    pub escalated: bool,
    pub required_role: Option<String>,
    pub required_tags: Vec<String>,
    pub scope_paths: Vec<String>,
    pub report_template: String,
    /// Set by `review_ticket` Approve; cleared on Reopened.
    pub approved: bool,
    /// Code tickets: the merge queue merged the branch. The ticket stays Resolved until `close_ticket`.
    pub merged: bool,
    /// The ticket branch carries commits (decided at submit time from the auto-commit).
    pub has_code: bool,
    pub close_reason: Option<String>,
    pub usage: UsageSummary,
    pub created_at: String,
    pub updated_at: String,
}

/// Reopened more than this many times sets the `escalated` flag (decision 3i).
pub const MAX_REOPEN_BEFORE_ESCALATE: u32 = 3;

/// `true` when `s` looks like a plan item id (`PI-3`).
pub fn is_plan_item_id(s: &str) -> bool {
    s.strip_prefix("PI-")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// `true` when `s` looks like an acceptance criterion id (`AC-3`).
pub fn is_ac_id(s: &str) -> bool {
    s.strip_prefix("AC-")
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// `true` when `s` is a template reference `name@version` (`coding@1`).
pub fn is_template_ref(s: &str) -> bool {
    s.split_once('@').is_some_and(|(name, v)| {
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
            && !v.is_empty()
            && v.bytes().all(|b| b.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips_as_string() {
        for s in TicketStatus::ALL {
            assert_eq!(s.as_str().parse::<TicketStatus>().unwrap(), s);
            assert_eq!(serde_json::to_string(&s).unwrap(), format!("\"{s}\""));
        }
        assert!(TicketStatus::Closed.is_terminal() && TicketStatus::Cancelled.is_terminal());
        assert!(!TicketStatus::Resolved.is_terminal());
    }

    #[test]
    fn id_shapes() {
        assert!(is_plan_item_id("PI-1") && !is_plan_item_id("PI-") && !is_plan_item_id("P1"));
        assert!(is_ac_id("AC-12") && !is_ac_id("ac-1"));
        assert!(is_template_ref("coding@1") && is_template_ref("pentest-findings@2"));
        assert!(
            !is_template_ref("Coding@1")
                && !is_template_ref("coding")
                && !is_template_ref("coding@x")
        );
    }
}
