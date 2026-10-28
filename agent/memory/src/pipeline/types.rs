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
    /// Lessons shown to the run that held again, in a new context.
    pub validated: usize,
    /// Forgetting and consolidation after the episode.
    pub decayed: u64,
    pub consolidated: u64,
    /// Facts, operations, mentions or relations refused by the guards.
    pub rejected: Vec<String>,
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
