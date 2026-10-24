//! Typed forms carried by protocol tool calls and stored as comment payloads.
//!
//! Doc comments become JSON Schema descriptions shown to models: keep them English, short and byte-stable
//! (prompt cache, decision 16). The idempotency key (`intent_id`) is not part of these forms: the harness derives
//! it from the run and the vendor tool-call id, so a replayed stream maps to the same intent (ADR-52).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::{AgentId, CommentId, TicketKey};
use crate::ticket::{is_ac_id, is_plan_item_id, is_template_ref, AcceptanceCriterion, TicketKind};
use crate::validate::{Checker, FormErrors, Validate};

// ---------------------------------------------------------------- worker forms

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AcStatus {
    Met,
    Partial,
    NotMet,
}

/// Result for one acceptance criterion of the ticket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AcResult {
    /// Acceptance criterion id, e.g. AC-1.
    pub ac_id: String,
    pub status: AcStatus,
    /// Where the evidence is: a test name, file path, command output or explanation.
    pub evidence: String,
}

/// A produced artifact (file, diff, commit, log, ...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
}

/// Final report of the work on a ticket. Moves the ticket InProgress -> Resolved and ends the run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkReport {
    pub ticket_id: TicketKey,
    /// What was done, at most 800 characters.
    #[schemars(length(max = 800))]
    pub summary: String,
    /// One result per acceptance criterion of the ticket (all of them).
    #[schemars(length(min = 1))]
    pub acceptance: Vec<AcResult>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    /// Template specific section, validated against the ticket's report template.
    #[serde(default = "empty_object")]
    #[schemars(with = "serde_json::Map<String, serde_json::Value>")]
    pub extension: serde_json::Value,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub follow_ups: Vec<String>,
    /// Your confidence that every criterion is met, 0..1.
    #[schemars(range(min = 0.0, max = 1.0))]
    pub confidence: f64,
}

fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(Default::default())
}

impl Validate for WorkReport {
    fn validate(&self) -> Result<(), FormErrors> {
        let mut c = Checker::new();
        c.len("summary", &self.summary, 1, 800);
        c.items("acceptance", &self.acceptance, 1, 200);
        let mut seen = std::collections::HashSet::new();
        for (i, a) in self.acceptance.iter().enumerate() {
            c.check(
                is_ac_id(&a.ac_id),
                format!("acceptance[{i}].ac_id"),
                "must look like AC-1",
            );
            c.check(
                seen.insert(a.ac_id.as_str()),
                format!("acceptance[{i}].ac_id"),
                "duplicate criterion",
            );
            c.non_blank(&format!("acceptance[{i}].evidence"), &a.evidence);
        }
        c.check(
            (0.0..=1.0).contains(&self.confidence),
            "confidence",
            "must be between 0 and 1",
        );
        c.check(self.extension.is_object(), "extension", "must be an object");
        c.finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum QuestionCategory {
    Requirement,
    Technical,
    Permission,
    /// Missing MCP/credential/skill, or work outside your role.
    Blocker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OptionProposal {
    pub label: String,
}

/// Ask the Lead (or the user) a question on the ticket. blocking=true pauses the ticket (Pending) until answered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub ticket_id: TicketKey,
    pub category: QuestionCategory,
    /// The question, 10-500 characters.
    #[schemars(length(min = 10, max = 500))]
    pub question: String,
    /// What you already checked, at most 1500 characters.
    #[serde(default)]
    #[schemars(length(max = 1500))]
    pub context: String,
    #[serde(default)]
    pub options: Vec<OptionProposal>,
    /// Index into options of the option you recommend.
    #[serde(default)]
    pub recommendation: Option<usize>,
    pub blocking: bool,
    #[serde(default)]
    pub urgency: Urgency,
}

impl Validate for Question {
    fn validate(&self) -> Result<(), FormErrors> {
        let mut c = Checker::new();
        c.len("question", &self.question, 10, 500);
        c.len("context", &self.context, 0, 1500);
        c.items("options", &self.options, 0, 10);
        if let Some(r) = self.recommendation {
            c.check(
                r < self.options.len(),
                "recommendation",
                "must be an index into options",
            );
        }
        for (i, o) in self.options.iter().enumerate() {
            c.non_blank(&format!("options[{i}].label"), &o.label);
        }
        c.finish()
    }
}

/// Post a progress update on the ticket; does not change its state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProgressUpdate {
    pub ticket_id: TicketKey,
    #[serde(default)]
    #[schemars(range(min = 0, max = 100))]
    pub percent: Option<u8>,
    /// Step k/n, e.g. "2/5".
    #[serde(default)]
    pub step: Option<String>,
    /// What you are doing now.
    pub current: String,
    #[serde(default)]
    pub next: Option<String>,
    #[serde(default)]
    pub eta: Option<String>,
}

impl Validate for ProgressUpdate {
    fn validate(&self) -> Result<(), FormErrors> {
        let mut c = Checker::new();
        c.len("current", &self.current, 1, 1000);
        if let Some(p) = self.percent {
            c.check(p <= 100, "percent", "must be 0..100");
        }
        c.finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PermissionKind {
    Network,
    Path,
    Command,
}

/// Ask for a permission the sandbox does not grant (network domain, path outside the worktree, blocked command).
/// Inside the project allowlist the Lead decides for this run only; outside it the user decides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PermissionRequest {
    pub ticket_id: TicketKey,
    pub kind: PermissionKind,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[schemars(length(min = 5))]
    pub reason: String,
}

impl Validate for PermissionRequest {
    fn validate(&self) -> Result<(), FormErrors> {
        let mut c = Checker::new();
        c.len("reason", &self.reason, 5, 1000);
        match self.kind {
            PermissionKind::Network => c.check(
                !self.domains.is_empty(),
                "domains",
                "required for kind=network",
            ),
            PermissionKind::Path => {
                c.check(!self.paths.is_empty(), "paths", "required for kind=path")
            }
            PermissionKind::Command => c.check(
                self.command
                    .as_deref()
                    .is_some_and(|s| !s.trim().is_empty()),
                "command",
                "required for kind=command",
            ),
        }
        c.finish()
    }
}

/// Propose a related_to link between two tickets; the Lead confirms it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuggestLink {
    pub from: TicketKey,
    pub to: TicketKey,
    pub reason: String,
}

impl Validate for SuggestLink {
    fn validate(&self) -> Result<(), FormErrors> {
        let mut c = Checker::new();
        c.check(self.from != self.to, "to", "must differ from `from`");
        c.non_blank("reason", &self.reason);
        c.finish()
    }
}

// ---------------------------------------------------------------- lead forms

/// Create a ticket for an approved plan item. depends_on must stay acyclic; overlapping scope_paths need depends_on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewTicket {
    pub kind: TicketKind,
    /// Plan item id, e.g. PI-1.
    #[schemars(pattern(r"^PI-[0-9]+$"))]
    pub plan_item: String,
    #[serde(default)]
    pub parent: Option<TicketKey>,
    #[schemars(length(min = 1, max = 200))]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[schemars(length(min = 1))]
    pub acceptance: Vec<AcceptanceCriterion>,
    /// 0 = most urgent .. 3.
    #[serde(default = "default_priority")]
    #[schemars(range(min = 0, max = 3))]
    pub priority: u8,
    #[serde(default)]
    pub depends_on: Vec<TicketKey>,
    #[serde(default)]
    pub required_role: Option<String>,
    #[serde(default)]
    pub required_tags: Vec<String>,
    /// Files/areas this ticket may touch (glob patterns relative to the project root).
    #[serde(default)]
    pub scope_paths: Vec<String>,
    /// Report template, e.g. coding@1. Defaults to the team template.
    #[serde(default)]
    #[schemars(pattern(r"^[a-z0-9_-]+@[0-9]+$"))]
    pub report_template: Option<String>,
}

fn default_priority() -> u8 {
    2
}

impl Validate for NewTicket {
    fn validate(&self) -> Result<(), FormErrors> {
        let mut c = Checker::new();
        c.check(
            is_plan_item_id(&self.plan_item),
            "plan_item",
            "must look like PI-1",
        );
        c.len("title", &self.title, 1, 200);
        c.non_blank("title", &self.title);
        c.items("acceptance", &self.acceptance, 1, 100);
        let mut seen = std::collections::HashSet::new();
        for (i, a) in self.acceptance.iter().enumerate() {
            c.check(
                is_ac_id(&a.id),
                format!("acceptance[{i}].id"),
                "must look like AC-1",
            );
            c.check(
                seen.insert(a.id.as_str()),
                format!("acceptance[{i}].id"),
                "duplicate id",
            );
            c.non_blank(&format!("acceptance[{i}].text"), &a.text);
        }
        c.check(self.priority <= 3, "priority", "must be 0..3");
        let mut deps = std::collections::HashSet::new();
        for (i, d) in self.depends_on.iter().enumerate() {
            c.check(
                deps.insert(d),
                format!("depends_on[{i}]"),
                "duplicate dependency",
            );
        }
        if let Some(t) = &self.report_template {
            c.check(
                is_template_ref(t),
                "report_template",
                "must look like name@1",
            );
        }
        for (i, p) in self.scope_paths.iter().enumerate() {
            c.check(
                !p.trim().is_empty() && !p.starts_with('/') && !p.contains(".."),
                format!("scope_paths[{i}]"),
                "must be a relative glob without '..'",
            );
        }
        c.finish()
    }
}

/// Assign a New/Reopened ticket to a worker of your team.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssignTicket {
    pub ticket_id: TicketKey,
    pub worker: AgentId,
    #[serde(default)]
    pub note: Option<String>,
}

/// Answer an open Question on a ticket, or (escalate = true) hand it to the user when you cannot decide it
/// (scope or acceptance changes, missing facts): `answer` then says what the user must decide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnswerQuestion {
    pub ticket_id: TicketKey,
    /// Id of the Question comment.
    pub reply_to: CommentId,
    #[schemars(length(min = 1))]
    pub answer: String,
    #[serde(default)]
    pub chosen_option: Option<usize>,
    /// true: do not answer; the question goes to the user with `answer` as your note.
    #[serde(default)]
    pub escalate: bool,
}

impl Validate for AnswerQuestion {
    fn validate(&self) -> Result<(), FormErrors> {
        let mut c = Checker::new();
        c.len("answer", &self.answer, 1, 4000);
        c.non_blank("answer", &self.answer);
        c.finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ReviewVerdict {
    Approve,
    Reopen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AcFeedback {
    pub ac_id: String,
    #[schemars(length(min = 1))]
    pub feedback: String,
}

/// Review a Resolved ticket. Approve sets approved=true (code tickets then enter the merge queue);
/// Reopen sends it back to the assignee and needs feedback for each criterion that is not met.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewDecision {
    pub ticket_id: TicketKey,
    pub decision: ReviewVerdict,
    #[serde(default)]
    pub ac_feedback: Vec<AcFeedback>,
    #[serde(default)]
    pub required_changes: Vec<String>,
}

impl Validate for ReviewDecision {
    fn validate(&self) -> Result<(), FormErrors> {
        let mut c = Checker::new();
        if self.decision == ReviewVerdict::Reopen {
            c.check(
                !self.ac_feedback.is_empty(),
                "ac_feedback",
                "Reopen needs feedback for each unmet criterion",
            );
        }
        for (i, f) in self.ac_feedback.iter().enumerate() {
            c.check(
                is_ac_id(&f.ac_id),
                format!("ac_feedback[{i}].ac_id"),
                "must look like AC-1",
            );
            c.non_blank(&format!("ac_feedback[{i}].feedback"), &f.feedback);
        }
        c.finish()
    }
}

/// Close a Resolved ticket (terminal). Code tickets must be merged first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CloseTicket {
    pub ticket_id: TicketKey,
    #[schemars(length(min = 5))]
    pub reason: String,
}

/// Cancel a ticket that is not finished; stops its run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancelTicket {
    pub ticket_id: TicketKey,
    #[schemars(length(min = 5))]
    pub reason: String,
}

fn validate_reason(reason: &str) -> Result<(), FormErrors> {
    let mut c = Checker::new();
    c.len("reason", reason.trim(), 5, 2000);
    c.finish()
}

impl Validate for CloseTicket {
    fn validate(&self) -> Result<(), FormErrors> {
        validate_reason(&self.reason)
    }
}

impl Validate for CancelTicket {
    fn validate(&self) -> Result<(), FormErrors> {
        validate_reason(&self.reason)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PermissionDecision {
    Approve,
    Deny,
}

/// Decide a pending permission request. Valid for the current run only; there is no "always" option.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApprovePermission {
    pub ticket_id: TicketKey,
    /// Id of the Permission comment.
    pub request_id: CommentId,
    pub decision: PermissionDecision,
    #[serde(default)]
    pub note: Option<String>,
}

/// Immutable handoff of selected context from a source ticket to a target ticket; creates a link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Handoff {
    pub source_ticket: TicketKey,
    pub target_ticket: TicketKey,
    #[schemars(length(min = 1))]
    pub source_items: Vec<ArtifactRef>,
    #[schemars(length(max = 2000))]
    pub summary: String,
    #[serde(default)]
    pub instruction: Option<String>,
}

impl Validate for Handoff {
    fn validate(&self) -> Result<(), FormErrors> {
        let mut c = Checker::new();
        c.check(
            self.source_ticket != self.target_ticket,
            "target_ticket",
            "must differ from source_ticket",
        );
        c.items("source_items", &self.source_items, 1, 50);
        c.len("summary", &self.summary, 1, 2000);
        c.finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    RelatedTo,
    Blocks,
    Duplicates,
    FollowUpOf,
    Aggregates,
}

impl LinkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RelatedTo => "related_to",
            Self::Blocks => "blocks",
            Self::Duplicates => "duplicates",
            Self::FollowUpOf => "follow_up_of",
            Self::Aggregates => "aggregates",
        }
    }
}

/// Confirm a link between two tickets (usually one proposed by a worker).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfirmLink {
    pub from: TicketKey,
    pub to: TicketKey,
    pub kind: LinkKind,
}

/// Propose a new role for the team; agents are created only after the user approves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposeRole {
    pub role_id: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub model_hint: Option<String>,
}

/// Give a member of your team (or yourself) its own key for an MCP server it uses, for example a key you
/// obtained for them through that server; `value` null removes it, so the member uses the system key again.
/// get_team lists each member's servers and key names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetMemberMcpKey {
    pub agent: AgentId,
    pub server: String,
    /// The key name: an env variable or HTTP header of the server (from get_team).
    pub key: String,
    #[serde(default)]
    pub value: Option<String>,
}

// ---------------------------------------------------------------- shared forms

/// Load the full SKILL.md of a skill listed in your skills index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoadSkill {
    pub name: String,
}

/// Propose a text change (unified diff) to your SOUL.md or a ROLE.md; the user approves it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposeChange {
    /// For role changes: the role id. Empty for SOUL.md.
    #[serde(default)]
    pub role_id: Option<String>,
    /// Unified diff.
    pub patch: String,
    pub reason: String,
}

/// Update your own overlay of a skill (decision 35).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposeSkillUpdate {
    pub skill: String,
    /// Unified diff against the current SKILL.md.
    pub patch: String,
    pub reason: String,
}

/// Answer stored on an Answer comment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Answer {
    pub resolution: String,
    #[serde(default)]
    pub chosen_option: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(json: serde_json::Value) -> Result<WorkReport, serde_json::Error> {
        serde_json::from_value(json)
    }

    #[test]
    fn work_report_accepts_design_example() {
        // From the architecture document ("Payload & ví dụ JSON"), minus intent_id (harness-assigned).
        let r = report(serde_json::json!({
            "ticket_id": "AGN-11", "summary": "Callback + session issuing done",
            "acceptance": [{ "ac_id": "AC-1", "status": "Met", "evidence": "tests/auth.rs::callback_ok" },
                           { "ac_id": "AC-2", "status": "Partial", "evidence": "refresh not tested" }],
            "artifacts": [{ "kind": "diff", "ref": "agenium/AGN-11" }],
            "extension": { "tests": { "ran": 12, "passed": 12 } },
            "risks": [], "follow_ups": ["Add refresh token test"], "confidence": 0.8
        }))
        .unwrap();
        r.validate().unwrap();
        assert_eq!(r.artifacts[0].reference, "agenium/AGN-11");
    }

    #[test]
    fn work_report_rejects_bad_fields() {
        assert!(report(serde_json::json!({"ticket_id":"AGN-1","summary":"x","acceptance":[],"confidence":0.5,"extra":1})).is_err());
        let r = report(serde_json::json!({"ticket_id":"AGN-1","summary":"","acceptance":[
            {"ac_id":"AC-1","status":"Met","evidence":" "},{"ac_id":"AC-1","status":"Met","evidence":"e"}],"confidence":1.5})).unwrap();
        let e = r.validate().unwrap_err();
        let fields: Vec<_> = e.0.iter().map(|f| f.field.as_str()).collect();
        assert_eq!(
            fields,
            [
                "summary",
                "acceptance[0].evidence",
                "acceptance[1].ac_id",
                "confidence"
            ]
        );
    }

    #[test]
    fn review_reopen_needs_feedback() {
        let r: ReviewDecision =
            serde_json::from_value(serde_json::json!({"ticket_id":"AGN-11","decision":"Reopen"}))
                .unwrap();
        assert!(r.validate().is_err());
        let r: ReviewDecision =
            serde_json::from_value(serde_json::json!({"ticket_id":"AGN-11","decision":"Reopen",
            "ac_feedback":[{"ac_id":"AC-2","feedback":"Add a test for refresh token expiry"}],
            "required_changes":["tests/auth.rs: refresh expiry case"]}))
            .unwrap();
        r.validate().unwrap();
    }

    #[test]
    fn question_and_ticket_examples() {
        let q: Question = serde_json::from_value(serde_json::json!({"ticket_id":"AGN-11","category":"Requirement",
            "question":"Which redirect URI should dev use?","context":"No value in spec or .env.example",
            "options":[{"label":"http://localhost:3000/cb"},{"label":"http://127.0.0.1:8080/cb"}],
            "recommendation":0,"blocking":true,"urgency":"High"})).unwrap();
        q.validate().unwrap();
        let t: NewTicket = serde_json::from_value(
            serde_json::json!({"kind":"Task","plan_item":"PI-1","parent":"AGN-10",
            "title":"Implement Google OAuth callback","description":"...",
            "acceptance":[{"id":"AC-1","text":"..."},{"id":"AC-2","text":"..."}],
            "priority":1,"depends_on":[],"required_role":"backend","required_tags":["rust"],
            "scope_paths":["src/auth/**"],"report_template":"coding@1"}),
        )
        .unwrap();
        t.validate().unwrap();
        let bad: NewTicket = serde_json::from_value(serde_json::json!({"kind":"Task","plan_item":"X","title":"t",
            "acceptance":[{"id":"AC-1","text":"a"}],"scope_paths":["../etc"],"depends_on":["AGN-1","AGN-1"]})).unwrap();
        assert_eq!(bad.validate().unwrap_err().0.len(), 3);
    }

    #[test]
    fn permission_request_requires_kind_specific_fields() {
        let p: PermissionRequest =
            serde_json::from_value(serde_json::json!({"ticket_id":"AGN-1","kind":"network",
            "reason":"cargo add oauth2"}))
            .unwrap();
        assert!(p.validate().is_err());
        let p: PermissionRequest =
            serde_json::from_value(serde_json::json!({"ticket_id":"AGN-1","kind":"network",
            "domains":["crates.io"],"reason":"cargo add oauth2"}))
            .unwrap();
        p.validate().unwrap();
    }
}
