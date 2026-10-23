//! Typed comments: every agent-to-agent/user communication lives on a ticket (decision 3d).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::forms::{
    Answer, Handoff, PermissionDecision, PermissionRequest, ProgressUpdate, Question,
    ReviewDecision, WorkReport,
};
use crate::ids::{AgentId, CommentId, RunId, TicketId, TicketKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CommentKind {
    Question,
    Answer,
    Progress,
    Report,
    Review,
    Permission,
    Handoff,
    System,
    User,
}

impl CommentKind {
    pub const ALL: [CommentKind; 9] = [
        Self::Question,
        Self::Answer,
        Self::Progress,
        Self::Report,
        Self::Review,
        Self::Permission,
        Self::Handoff,
        Self::System,
        Self::User,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Question => "Question",
            Self::Answer => "Answer",
            Self::Progress => "Progress",
            Self::Report => "Report",
            Self::Review => "Review",
            Self::Permission => "Permission",
            Self::Handoff => "Handoff",
            Self::System => "System",
            Self::User => "User",
        }
    }
}

impl fmt::Display for CommentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CommentKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| format!("unknown comment kind {s:?}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Author {
    Agent(AgentId),
    User,
    System,
}

impl fmt::Display for Author {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Author::Agent(a) => write!(f, "agent:{a}"),
            Author::User => f.write_str("user"),
            Author::System => f.write_str("system"),
        }
    }
}

impl FromStr for Author {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(Author::User),
            "system" => Ok(Author::System),
            _ => s
                .strip_prefix("agent:")
                .and_then(|a| a.parse().ok())
                .map(Author::Agent)
                .ok_or_else(|| format!("invalid author {s:?}")),
        }
    }
}

impl Serialize for Author {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Author {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Decision recorded on a Permission comment once someone decided it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionOutcome {
    pub decision: PermissionDecision,
    /// `user` or `agent:<id>`.
    pub decided_by: String,
    /// `run` (Lead grants) or `project` (always-allow, user only).
    pub scope: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// Payload of a comment, by kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum CommentBody {
    Question(Question),
    Answer(Answer),
    Progress(ProgressUpdate),
    Report(WorkReport),
    Review(ReviewDecision),
    /// `run` = the run that asked (audit; run-scoped grants are tied to the ticket's current work cycle).
    Permission {
        request: PermissionRequest,
        #[serde(default)]
        run: Option<RunId>,
        outcome: Option<PermissionOutcome>,
    },
    Handoff(Handoff),
    /// System notices (state changes, merge results, escalation, nudges).
    System {
        text: String,
    },
    /// Free text from the user, or from an agent posting plain text.
    Text {
        text: String,
    },
}

impl CommentBody {
    pub fn kind(&self) -> CommentKind {
        match self {
            CommentBody::Question(_) => CommentKind::Question,
            CommentBody::Answer(_) => CommentKind::Answer,
            CommentBody::Progress(_) => CommentKind::Progress,
            CommentBody::Report(_) => CommentKind::Report,
            CommentBody::Review(_) => CommentKind::Review,
            CommentBody::Permission { .. } => CommentKind::Permission,
            CommentBody::Handoff(_) => CommentKind::Handoff,
            CommentBody::System { .. } => CommentKind::System,
            CommentBody::Text { .. } => CommentKind::User,
        }
    }

    /// Whether this comment, while unresolved, keeps the ticket Pending.
    pub fn is_blocking(&self) -> bool {
        match self {
            CommentBody::Question(q) => q.blocking,
            CommentBody::Permission { outcome, .. } => outcome.is_none(),
            _ => false,
        }
    }
}

/// A stored comment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub id: CommentId,
    pub ticket: TicketId,
    pub ticket_key: TicketKey,
    pub kind: CommentKind,
    pub author: Author,
    pub reply_to: Option<CommentId>,
    pub body: CommentBody,
    pub blocking: bool,
    pub resolved: bool,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn author_round_trip() {
        let a = AgentId::new();
        for x in [Author::Agent(a), Author::User, Author::System] {
            assert_eq!(x.to_string().parse::<Author>().unwrap(), x);
        }
        assert!("agent:nope".parse::<Author>().is_err());
    }

    #[test]
    fn body_kind_and_blocking() {
        let b = CommentBody::System { text: "x".into() };
        assert_eq!(b.kind(), CommentKind::System);
        assert!(!b.is_blocking());
        let json = serde_json::to_value(&b).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"type":"System","data":{"text":"x"}})
        );
    }
}
