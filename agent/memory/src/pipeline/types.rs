//! What learning takes and gives: the episode, the models it uses, the report, and the extraction's parts.

use super::*;

/// What happened, as the agent saw it (no thinking blocks).
#[derive(Debug, Clone, Default)]
pub struct Episode {
    /// `ticket:<KEY>` or `chat`.
    pub source: String,
    /// Project id (the fact `project` column).
    pub project: Option<String>,
    /// Project name, the default scope of lessons.
    pub project_name: Option<String>,
    /// "Closed: ..." / "Reopened: <feedback>" / "chat".
    pub outcome: String,
    pub reopened: bool,
    /// Ticket, report, review feedback and the run conversation as plain text.
    pub transcript: String,
    /// Run whose work order showed `used` (reuse signals).
    pub run: Option<String>,
    /// Memories shown to that run: (fact id, text).
    pub used: Vec<(i64, String)>,
    /// Lessons of the shared pool (ADR-87) shown to that run: (pool fact id, text). The pipeline never changes them:
    /// those a reopened run names as misleading are reported (`LearnReport::misleading_shared`) for the pool to weigh
    /// (audit M3, 2026-10-31).
    pub shared_used: Vec<(i64, String)>,
    /// What makes the episode worth a lesson (decision 35): a reopen, a failed merge, the Lead's correction or
    /// praise, the user's feedback. Without one only facts and preferences are kept.
    pub signals: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PipelineModel {
    pub model: String,
    pub effort: Effort,
    pub max_tokens: u32,
    pub retry: RetryPolicy,
}

/// One change an episode made to the memory, in words: a fact, preference or lesson added, rewritten or retired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    /// `add`, `update` or `delete`.
    pub op: &'static str,
    pub kind: FactKind,
    pub text: String,
    /// The text an update replaced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub was: Option<String>,
    /// The fact this change wrote (add, update), so the episode can be undone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// The fact it replaced (update) or retired (delete).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_id: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct LearnReport {
    pub extracted: usize,
    pub added: usize,
    pub updated: usize,
    pub deleted: usize,
    pub noop: usize,
    pub entities_new: usize,
    pub entities_linked: usize,
    pub edges: usize,
    pub helped: usize,
    pub misleading: Vec<i64>,
    /// Pool fact ids of the shared lessons named as misleading (`Episode::shared_used`): never this agent's facts.
    pub misleading_shared: Vec<i64>,
    /// Lessons shown to the run that held again, in a new context.
    pub validated: usize,
    /// Forgetting and consolidation after the episode.
    pub decayed: u64,
    pub consolidated: u64,
    /// Facts, operations, mentions or relations refused by the guards.
    pub rejected: Vec<String>,
    /// A step that failed after the episode's reuse signals were recorded: what it lost (the new facts).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<String>,
    /// What the counts above changed, in words (the run log shows it).
    pub changes: Vec<Change>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    /// The model calls' usage, for pricing them (the caller records it as a run).
    #[serde(skip)]
    pub usage: Usage,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct Mention {
    pub(super) name: String,
    #[serde(rename = "type", default)]
    pub(super) kind: String,
    #[serde(default)]
    pub(super) role: Option<String>,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) scope: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct Relation {
    pub(super) src: String,
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) dst: String,
    #[serde(default)]
    pub(super) scope: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct Extracted {
    pub(super) text: String,
    #[serde(default)]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) external: bool,
    #[serde(default)]
    pub(super) cue: Option<String>,
    #[serde(default)]
    pub(super) scope: Value,
    #[serde(default)]
    pub(super) entities: Vec<Mention>,
    #[serde(default)]
    pub(super) relations: Vec<Relation>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "UPPERCASE")]
pub(super) enum Op {
    Add {
        fact: usize,
        #[serde(default)]
        text: Option<String>,
    },
    Update {
        fact: usize,
        id: i64,
        #[serde(default)]
        text: Option<String>,
    },
    Delete {
        id: i64,
        /// The new fact that contradicts the memory: its evidence.
        #[serde(default)]
        fact: Option<usize>,
    },
    Noop {
        #[serde(default)]
        #[allow(dead_code)]
        fact: Option<usize>,
    },
}
