//! Agent <-> system protocol (decision 36): the closed catalogue of protocol tools, which role may call which,
//! and the typed intents they produce. AI never writes state directly: a protocol tool call becomes an
//! [`Intent`] that the orchestrator validates and commits.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::agent::RoleKind;
use crate::forms::*;
use crate::plan::{PlanDraft, PlanRevision, RequirementSpec, TicketPlan};
use crate::validate::{FormErrors, Validate};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolGroup {
    Lead,
    Worker,
    Shared,
}

macro_rules! protocol_tools {
    ($($variant:ident => $name:literal, $group:ident, $args:ty, $desc:literal;)*) => {
        /// Every protocol tool. There is no `split_ticket` (use `revise_plan`) and no `request_reassign`
        /// (use `ask_question` with category Blocker).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum ProtocolTool { $($variant,)* }

        impl ProtocolTool {
            pub const ALL: &'static [ProtocolTool] = &[$(ProtocolTool::$variant,)*];

            pub fn name(self) -> &'static str {
                match self { $(ProtocolTool::$variant => $name,)* }
            }

            pub fn group(self) -> ToolGroup {
                match self { $(ProtocolTool::$variant => ToolGroup::$group,)* }
            }

            /// Model-facing description: English, short, byte-stable (prompt cache).
            pub fn description(self) -> &'static str {
                match self { $(ProtocolTool::$variant => $desc,)* }
            }

            /// JSON Schema (draft 2020-12, subschemas inlined) of the tool's arguments.
            pub fn parameters(self) -> serde_json::Value {
                match self { $(ProtocolTool::$variant => args_schema::<$args>(),)* }
            }
        }

        impl FromStr for ProtocolTool {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s { $($name => Ok(ProtocolTool::$variant),)* _ => Err(format!("unknown protocol tool {s:?}")) }
            }
        }
    };
}

/// Argument schema as sent to models: inlined (no `$ref`/`$defs`), no `$schema`/`title` noise.
pub fn args_schema<T: schemars::JsonSchema>() -> serde_json::Value {
    let mut settings = schemars::generate::SchemaSettings::draft2020_12();
    settings.inline_subschemas = true;
    settings.meta_schema = None;
    let gen = settings.into_generator();
    let mut v = gen.into_root_schema_for::<T>().to_value();
    if let Some(o) = v.as_object_mut() {
        o.remove("title");
        o.remove("$schema");
    }
    v
}

protocol_tools! {
    ProposeSpec => "propose_spec", Lead, RequirementSpec,
        "Propose the requirement spec after discovery. A team's spec waits for user approval; a Solo agent's spec is locked at once and the user is informed.";
    CreatePlan => "create_plan", Lead, TicketPlan,
        "Create plan v1 from the locked spec: milestones and items, each item tracing to requirement ids. Every ticket must reference a plan item.";
    RevisePlan => "revise_plan", Lead, PlanRevision,
        "Revise the plan (version+1, with a reason). Use it instead of splitting a ticket: add items, then create tickets for them. Scope changes need user approval.";
    CreateTicket => "create_ticket", Lead, NewTicket,
        "Create a ticket for an item of the approved plan. depends_on must stay acyclic; tickets whose scope_paths overlap an open ticket must depend on it.";
    AssignTicket => "assign_ticket", Lead, AssignTicket,
        "Assign a New or Reopened ticket to a worker of your team whose role and tags match. The scheduler starts the work when dependencies are satisfied.";
    AnswerQuestion => "answer_question", Lead, AnswerQuestion,
        "Answer an open Question comment on a ticket of your team. If you are not sure, ask the user instead of guessing.";
    ReviewTicket => "review_ticket", Lead, ReviewDecision,
        "Review a Resolved ticket. Approve marks it approved (code tickets then enter the merge queue); Reopen sends it back with feedback per unmet acceptance criterion.";
    CloseTicket => "close_ticket", Lead, CloseTicket,
        "Close a Resolved, approved ticket with a reason. Code tickets must be merged first. Closed is final.";
    CancelTicket => "cancel_ticket", Lead, CancelTicket,
        "Cancel a ticket that is not finished, with a reason; its run stops.";
    CreateHandoff => "create_handoff", Lead, Handoff,
        "Hand selected context from a source ticket to a target ticket as an immutable Handoff comment; links the tickets.";
    ApprovePermission => "approve_permission", Lead, ApprovePermission,
        "Decide a permission request inside the project allowlist, for the current work on that ticket only. Requests outside the allowlist go to the user.";
    ProposeRole => "propose_role", Lead, ProposeRole,
        "Propose a role your team lacks: a role id from get_team's role_library, or a new id with what the role does and why the plan needs it. An agent with it joins your team only if the user approves.";
    GetTeam => "get_team", Lead, NoArgs,
        "Read the current team roster: members, roles, tags, model, status, current ticket and MCP servers with their key names.";
    SetMemberMcpKey => "set_member_mcp_key", Lead, SetMemberMcpKey,
        "Give a member its own key for an MCP server it uses (value null: back to the system key). Never put the key anywhere else.";
    ConfirmLink => "confirm_link", Lead, ConfirmLink,
        "Confirm a link between two tickets (usually one a worker proposed).";
    CreateAggregate => "create_aggregate", Lead, CreateAggregate,
        "Create an aggregate ticket (for example an attack chain) linked to two or more existing tickets.";
    PostProgress => "post_progress", Worker, ProgressUpdate,
        "Post a short progress update on your ticket; does not change its state.";
    AskQuestion => "ask_question", Worker, Question,
        "Ask a question on your ticket. blocking=true pauses the ticket until it is answered. Use category Blocker for missing MCP/credentials/skills or work outside your role.";
    RequestPermission => "request_permission", Worker, PermissionRequest,
        "Request a permission the sandbox does not give (network domain, path outside the worktree, blocked command). The ticket pauses until decided.";
    SubmitReport => "submit_report", Worker, WorkReport,
        "Submit the final report of your ticket with one result per acceptance criterion. Your worktree is committed and the ticket becomes Resolved. This ends your run.";
    SuggestLink => "suggest_link", Worker, SuggestLink,
        "Suggest that your ticket is related to another ticket; the Lead decides.";
    SubmitPlanDraft => "submit_plan_draft", Worker, PlanDraft,
        "Planner only: submit your draft of plan v1 for the locked spec, with the rationale of the split. The Lead reviews it and creates the plan. This ends your run.";
    LoadSkill => "load_skill", Shared, LoadSkill,
        "Load the full instructions of a skill listed in your skills index.";
    ProposeSkillUpdate => "propose_skill_update", Shared, ProposeSkillUpdate,
        "Propose an update to your own copy of a skill, backed by a lesson that held in two contexts: either the complete new body as content (simplest), or a unified diff as patch.";
    ProposeNewSkill => "propose_new_skill", Shared, ProposeNewSkill,
        "Create a new skill in your own skills folder from a lesson that held in two contexts. Only when no skill you have covers it (otherwise use propose_skill_update); it must have every section filled (Job, When to use, Steps, Tools, Proof). Checked strictly; the user can review or delete it.";
    ProposeSoulChange => "propose_soul_change", Shared, ProposeChange,
        "Propose a change to your SOUL.md; the user decides.";
    ProposeRoleChange => "propose_role_change", Shared, ProposeChange,
        "Propose a change to a ROLE.md; the user decides.";
}

impl fmt::Display for ProtocolTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl ProtocolTool {
    /// Tool-per-role rule: Lead = Lead + Shared, Worker = Worker + Shared, Solo = all.
    pub fn allowed(self, role: RoleKind, solo: bool) -> bool {
        match (self.group(), role) {
            (ToolGroup::Shared, _)
            | (ToolGroup::Lead, RoleKind::Lead)
            | (ToolGroup::Worker, RoleKind::Worker) => true,
            (ToolGroup::Worker, RoleKind::Lead) => solo,
            (ToolGroup::Lead, RoleKind::Worker) => false,
        }
    }

    /// Tools that end a worker run when they succeed (submit) or park it (blocking question handled separately).
    pub fn ends_run(self) -> bool {
        matches!(self, ProtocolTool::SubmitReport)
    }

    pub fn for_role(role: RoleKind, solo: bool) -> Vec<ProtocolTool> {
        Self::ALL
            .iter()
            .copied()
            .filter(|t| t.allowed(role, solo))
            .collect()
    }
}

/// Aggregate ticket form (e.g. an attack chain).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAggregate {
    #[schemars(length(min = 2))]
    pub children: Vec<crate::ids::TicketKey>,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
}

/// get_team takes no arguments.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

/// A validated-by-shape protocol call. Semantic rules (state machine, ownership, plan) are checked by the
/// orchestrator against the database.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tool", content = "args", rename_all = "snake_case")]
pub enum Intent {
    ProposeSpec(RequirementSpec),
    CreatePlan(TicketPlan),
    RevisePlan(PlanRevision),
    CreateTicket(NewTicket),
    AssignTicket(AssignTicket),
    AnswerQuestion(AnswerQuestion),
    ReviewTicket(ReviewDecision),
    CloseTicket(CloseTicket),
    CancelTicket(CancelTicket),
    CreateHandoff(Handoff),
    ApprovePermission(ApprovePermission),
    ProposeRole(ProposeRole),
    GetTeam(NoArgs),
    SetMemberMcpKey(SetMemberMcpKey),
    ConfirmLink(ConfirmLink),
    CreateAggregate(CreateAggregate),
    PostProgress(ProgressUpdate),
    AskQuestion(Question),
    RequestPermission(PermissionRequest),
    SubmitReport(WorkReport),
    SuggestLink(SuggestLink),
    SubmitPlanDraft(PlanDraft),
    LoadSkill(LoadSkill),
    ProposeSkillUpdate(ProposeSkillUpdate),
    ProposeNewSkill(ProposeNewSkill),
    ProposeSoulChange(ProposeChange),
    ProposeRoleChange(ProposeChange),
}

/// Why a protocol call could not even be parsed.
#[derive(Debug, Clone, PartialEq, thiserror::Error, Serialize)]
#[serde(tag = "code")]
pub enum IntentError {
    #[error("unknown tool {name:?}")]
    UnknownTool { name: String },
    #[error("tool {tool} is not available to your role")]
    NotAllowed { tool: String },
    #[error("invalid arguments for {tool}: {message}")]
    Schema { tool: String, message: String },
    #[error("invalid arguments for {tool}: {errors}")]
    Fields { tool: String, errors: FormErrors },
}

impl Intent {
    pub fn tool(&self) -> ProtocolTool {
        use Intent::*;
        match self {
            ProposeSpec(_) => ProtocolTool::ProposeSpec,
            CreatePlan(_) => ProtocolTool::CreatePlan,
            RevisePlan(_) => ProtocolTool::RevisePlan,
            CreateTicket(_) => ProtocolTool::CreateTicket,
            AssignTicket(_) => ProtocolTool::AssignTicket,
            AnswerQuestion(_) => ProtocolTool::AnswerQuestion,
            ReviewTicket(_) => ProtocolTool::ReviewTicket,
            CloseTicket(_) => ProtocolTool::CloseTicket,
            CancelTicket(_) => ProtocolTool::CancelTicket,
            CreateHandoff(_) => ProtocolTool::CreateHandoff,
            ApprovePermission(_) => ProtocolTool::ApprovePermission,
            ProposeRole(_) => ProtocolTool::ProposeRole,
            GetTeam(_) => ProtocolTool::GetTeam,
            SetMemberMcpKey(_) => ProtocolTool::SetMemberMcpKey,
            ConfirmLink(_) => ProtocolTool::ConfirmLink,
            CreateAggregate(_) => ProtocolTool::CreateAggregate,
            PostProgress(_) => ProtocolTool::PostProgress,
            AskQuestion(_) => ProtocolTool::AskQuestion,
            RequestPermission(_) => ProtocolTool::RequestPermission,
            SubmitReport(_) => ProtocolTool::SubmitReport,
            SuggestLink(_) => ProtocolTool::SuggestLink,
            SubmitPlanDraft(_) => ProtocolTool::SubmitPlanDraft,
            LoadSkill(_) => ProtocolTool::LoadSkill,
            ProposeSkillUpdate(_) => ProtocolTool::ProposeSkillUpdate,
            ProposeNewSkill(_) => ProtocolTool::ProposeNewSkill,
            ProposeSoulChange(_) => ProtocolTool::ProposeSoulChange,
            ProposeRoleChange(_) => ProtocolTool::ProposeRoleChange,
        }
    }

    /// Parses a tool call (`name`, JSON `args`) for an agent with the given role, then runs field validation.
    pub fn parse(
        name: &str,
        args: serde_json::Value,
        role: RoleKind,
        solo: bool,
    ) -> Result<Intent, IntentError> {
        let tool: ProtocolTool = name.parse().map_err(|_| IntentError::UnknownTool {
            name: name.to_owned(),
        })?;
        if !tool.allowed(role, solo) {
            return Err(IntentError::NotAllowed {
                tool: name.to_owned(),
            });
        }
        // Models sometimes send `null` for "no arguments".
        let args = if args.is_null() {
            serde_json::json!({})
        } else {
            args
        };
        let intent: Intent = serde_json::from_value(
            serde_json::json!({ "tool": name, "args": args }),
        )
        .map_err(|e| IntentError::Schema {
            tool: name.to_owned(),
            message: e.to_string(),
        })?;
        intent.validate().map_err(|errors| IntentError::Fields {
            tool: name.to_owned(),
            errors,
        })?;
        Ok(intent)
    }

    pub fn validate(&self) -> Result<(), FormErrors> {
        use Intent::*;
        match self {
            ProposeSpec(x) => x.validate(),
            CreatePlan(x) => x.validate(),
            RevisePlan(x) => x.validate(),
            CreateTicket(x) => x.validate(),
            AnswerQuestion(x) => x.validate(),
            ReviewTicket(x) => x.validate(),
            CloseTicket(x) => x.validate(),
            CancelTicket(x) => x.validate(),
            CreateHandoff(x) => x.validate(),
            PostProgress(x) => x.validate(),
            AskQuestion(x) => x.validate(),
            RequestPermission(x) => x.validate(),
            SubmitReport(x) => x.validate(),
            SuggestLink(x) => x.validate(),
            SubmitPlanDraft(x) => x.validate(),
            AssignTicket(_)
            | ApprovePermission(_)
            | ProposeRole(_)
            | GetTeam(_)
            | SetMemberMcpKey(_)
            | ConfirmLink(_)
            | CreateAggregate(_)
            | LoadSkill(_)
            | ProposeSkillUpdate(_)
            | ProposeNewSkill(_)
            | ProposeSoulChange(_)
            | ProposeRoleChange(_) => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_matches_design_index() {
        // docs/design/SCHEMAS/tools/index.json: Lead 15, Worker 5, Shared 4 (24 tools); Solo = all. Agenium adds
        // set_member_mcp_key (Lead group; per-agent MCP keys, 2026-10-25) and
        // submit_plan_draft (Worker group, planner role only; ADR-59) and propose_new_skill (Shared; ADR-91).
        let count = |g| ProtocolTool::ALL.iter().filter(|t| t.group() == g).count();
        assert_eq!(
            (
                count(ToolGroup::Lead),
                count(ToolGroup::Worker),
                count(ToolGroup::Shared)
            ),
            (16, 6, 5)
        );
        assert_eq!(ProtocolTool::for_role(RoleKind::Lead, true).len(), 27);
        assert_eq!(ProtocolTool::for_role(RoleKind::Lead, false).len(), 21);
        assert_eq!(ProtocolTool::for_role(RoleKind::Worker, false).len(), 11);
        for t in ProtocolTool::ALL {
            assert_eq!(t.name().parse::<ProtocolTool>().unwrap(), *t);
        }
        assert!("split_ticket".parse::<ProtocolTool>().is_err());
        assert!("request_reassign".parse::<ProtocolTool>().is_err());
    }

    #[test]
    fn every_tool_has_an_inline_object_schema_and_description() {
        for t in ProtocolTool::ALL {
            let v = t.parameters();
            assert_eq!(v["type"], "object", "{t}");
            let text = v.to_string();
            assert!(
                !text.contains("$ref") && !text.contains("$defs"),
                "{t}: {text}"
            );
            assert!(
                t.description().len() > 20 && t.description().is_ascii(),
                "{t}"
            );
        }
        let sr = ProtocolTool::SubmitReport.parameters();
        assert_eq!(sr["additionalProperties"], false);
        let req: Vec<&str> = sr["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(req, ["ticket_id", "summary", "acceptance", "confidence"]);
        assert_eq!(
            sr["properties"]["ticket_id"]["pattern"],
            "^[A-Z]{1,10}-[1-9][0-9]*$"
        );
    }

    #[test]
    fn parse_checks_role_schema_and_fields() {
        let args = serde_json::json!({"ticket_id":"AGN-1","reason":"All AC met, merged"});
        let e = Intent::parse("close_ticket", args.clone(), RoleKind::Worker, false).unwrap_err();
        assert!(matches!(e, IntentError::NotAllowed { .. }));
        let i = Intent::parse("close_ticket", args, RoleKind::Lead, false).unwrap();
        assert_eq!(i.tool(), ProtocolTool::CloseTicket);
        let e = Intent::parse(
            "close_ticket",
            serde_json::json!({"ticket_id":"AGN-1"}),
            RoleKind::Lead,
            false,
        )
        .unwrap_err();
        assert!(matches!(e, IntentError::Schema { .. }), "{e}");
        let e = Intent::parse(
            "close_ticket",
            serde_json::json!({"ticket_id":"AGN-1","reason":"ok"}),
            RoleKind::Lead,
            false,
        )
        .unwrap_err();
        assert!(matches!(e, IntentError::Fields { .. }), "{e}");
        assert!(Intent::parse("get_team", serde_json::Value::Null, RoleKind::Lead, false).is_ok());
        assert!(matches!(
            Intent::parse("rm_rf", serde_json::json!({}), RoleKind::Lead, true),
            Err(IntentError::UnknownTool { .. })
        ));
    }

    /// M3-3 acceptance: every JSON example of the architecture document ("Payload & ví dụ JSON") passes the tool
    /// schema and the field rules, for the role the document gives it. `intent_id` is assigned by the harness
    /// (not a model argument) and the elided worker id is a real agent id here.
    #[test]
    fn design_document_examples_parse() {
        let worker = crate::ids::AgentId::new().to_string();
        let examples = [
            (
                "create_plan",
                RoleKind::Lead,
                serde_json::json!({ "spec_version": 1,
              "milestones": [{ "id": "M1", "title": "Google login" }],
              "items": [
                { "id": "PI-1", "milestone": "M1", "title": "OAuth backend", "trace_to": ["REQ-2"],
                  "acceptance": [{ "id": "AC-1", "text": "Login via Google returns a session" }],
                  "role": "backend", "depends_on": [], "estimate": "M" },
                { "id": "PI-2", "milestone": "M1", "title": "Login button", "trace_to": ["REQ-2"],
                  "acceptance": [{ "id": "AC-1", "text": "Button starts OAuth flow" }],
                  "role": "frontend", "depends_on": ["PI-1"], "estimate": "S" } ] }),
            ),
            (
                "create_ticket",
                RoleKind::Lead,
                serde_json::json!({ "kind": "Task", "plan_item": "PI-1", "parent": "AGN-10",
              "title": "Implement Google OAuth callback", "description": "...",
              "acceptance": [{ "id": "AC-1", "text": "..." }, { "id": "AC-2", "text": "..." }],
              "priority": 1, "depends_on": [], "required_role": "backend", "required_tags": ["rust"],
              "scope_paths": ["src/auth/**"], "report_template": "coding@1" }),
            ),
            (
                "assign_ticket",
                RoleKind::Lead,
                serde_json::json!({ "ticket_id": "AGN-11", "worker": worker, "note": "Start with AC-1" }),
            ),
            (
                "ask_question",
                RoleKind::Worker,
                serde_json::json!({ "ticket_id": "AGN-11", "category": "Requirement",
              "question": "Which redirect URI should dev use?", "context": "No value in spec or .env.example",
              "options": [{ "label": "http://localhost:3000/cb" }, { "label": "http://127.0.0.1:8080/cb" }],
              "recommendation": 0, "blocking": true, "urgency": "High" }),
            ),
            (
                "submit_report",
                RoleKind::Worker,
                serde_json::json!({ "ticket_id": "AGN-11", "summary": "Callback + session issuing done",
              "acceptance": [{ "ac_id": "AC-1", "status": "Met", "evidence": "tests/auth.rs::callback_ok" },
                             { "ac_id": "AC-2", "status": "Partial", "evidence": "refresh not tested" }],
              "artifacts": [{ "kind": "diff", "ref": "agenium/AGN-11" }],
              "extension": { "tests": { "ran": 12, "passed": 12 } },
              "risks": [], "follow_ups": ["Add refresh token test"], "confidence": 0.8 }),
            ),
            (
                "review_ticket",
                RoleKind::Lead,
                serde_json::json!({ "ticket_id": "AGN-11", "decision": "Reopen",
              "ac_feedback": [{ "ac_id": "AC-2", "feedback": "Add a test for refresh token expiry" }],
              "required_changes": ["tests/auth.rs: refresh expiry case"] }),
            ),
            (
                "close_ticket",
                RoleKind::Lead,
                serde_json::json!({ "ticket_id": "AGN-11", "reason": "All AC met, merged into main" }),
            ),
        ];
        for (tool, role, args) in examples {
            let i =
                Intent::parse(tool, args, role, false).unwrap_or_else(|e| panic!("{tool}: {e}"));
            assert_eq!(i.tool().name(), tool);
        }
    }
}
