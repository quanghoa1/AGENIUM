//! The intent engine: validates a protocol call against the database (role, ownership, state machine, plan,
//! dependency DAG, scope) and commits its state changes, comments and events in one transaction, idempotent by
//! intent id. Git side effects are journaled in the same transaction and executed right after (see `effects`).

use std::collections::{HashMap, HashSet};

use agenium_core::forms::*;
use agenium_core::plan::PlanRevision;
use agenium_core::protocol::CreateAggregate;
use agenium_core::*;
use agenium_store::core::plans::{PlanRow, SpecRow};
use agenium_store::core::{agents, comments, plans, projects, runs, tickets, workspaces, WriteCtx};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::effects::Op;
use crate::engine_support::*;
use crate::{OrchError, Result};

// Helpers other modules reach as `crate::engine::…`.
pub(crate) use crate::engine_support::{
    allowlist, deps_satisfied, in_allowlist, system_comment, transition,
};

// The intents, by concern (split out of this file; behaviour unchanged).
mod links;
mod permissions;
mod planning;
mod questions;
mod reports;
mod team;
mod ticket_intents;
mod ticket_reads;
mod update_main;
mod write_scope;

/// Who calls, for which project, under which idempotency key and run.
#[derive(Debug, Clone)]
pub struct CallCtx {
    pub project: ProjectId,
    pub actor: Actor,
    pub intent_id: Option<String>,
    pub run: Option<RunId>,
}

impl CallCtx {
    pub fn user(project: ProjectId) -> Self {
        Self {
            project,
            actor: Actor::User,
            intent_id: None,
            run: None,
        }
    }
}

/// What the agent runtime should do with the current run after a successful call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunEffect {
    Continue,
    /// The run is finished (work submitted).
    End,
    /// The ticket waits for an answer/approval; the run stops and a continuation run starts once unblocked.
    Park,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub result: serde_json::Value,
    pub run: RunEffect,
    /// Journaled git operations committed with the intent and executed right after it.
    #[serde(default)]
    pub ops: Vec<i64>,
}

impl Outcome {
    fn ok(result: serde_json::Value) -> Self {
        Self {
            result,
            run: RunEffect::Continue,
            ops: vec![],
        }
    }
}

/// What became of the user's answer to a question (ADR-122).
#[derive(Debug, Clone, PartialEq)]
pub enum UserAnswer {
    /// Answered on the ticket (a Solo agent's question).
    Answered(Outcome),
    /// A message for the Lead's chat: the Lead answers on the ticket.
    ToLead(String),
}

impl App {
    /// Applies a protocol intent. Errors are structured (see [`OrchError::to_tool_error`]) and change nothing.
    pub fn apply(&self, ctx: &CallCtx, intent: Intent) -> Result<Outcome> {
        let project = self.project(ctx.project)?;
        // An intent committed already (the same call sent again: a retry after its answer was lost) returns its stored
        // result before its handler's checks, which the first commit changed the state against: a report sent again
        // failed with "not in progress" (audit L22, 2026-10-31). Its git operations ran after the first commit. One
        // that parks the run (a permission, a blocking question) goes through its handler, which knows if it still
        // waits.
        let parks = matches!(
            &intent,
            Intent::RequestPermission(_) | Intent::AskQuestion(_)
        );
        if let (Some(id), false) = (ctx.intent_id.as_deref(), parks) {
            let stored = self
                .store
                .intent_result(id)?
                .and_then(|v| serde_json::from_value::<Outcome>(v).ok());
            if let Some(o) = stored {
                return Ok(Outcome { ops: vec![], ..o });
            }
        }
        let reviewed = match &intent {
            Intent::ReviewTicket(r) => Some(r.ticket_id.clone()),
            _ => None,
        };
        let mut out = match intent {
            Intent::CreateTicket(nt) => self.create_ticket(ctx, &project, nt),
            Intent::AssignTicket(a) => self.assign_ticket(ctx, a),
            Intent::AnswerQuestion(a) => self.answer_question(ctx, a),
            Intent::ReviewTicket(r) => self.review_ticket(ctx, &project, r),
            Intent::CloseTicket(c) => self.close_ticket(ctx, c),
            Intent::CancelTicket(c) => self.cancel_ticket(ctx, c),
            Intent::CreateHandoff(h) => self.create_handoff(ctx, h),
            Intent::ApprovePermission(a) => self.approve_permission(ctx, &project, a),
            Intent::GetTeam(_) => self.get_team(ctx),
            Intent::ListTickets(q) => self.list_tickets(ctx, q),
            Intent::ReadTicket(q) => self.read_ticket(ctx, q),
            Intent::ReadSpec(_) => self.read_spec(ctx),
            Intent::SetMemberMcpKey(k) => self.set_member_mcp_key(ctx, k),
            Intent::ConfirmLink(l) => self.confirm_link(ctx, l),
            Intent::CreateAggregate(a) => self.create_aggregate(ctx, &project, a),
            Intent::PostProgress(p) => self.post_progress(ctx, p),
            Intent::AskQuestion(q) => self.ask_question(ctx, q),
            Intent::RequestPermission(r) => self.request_permission(ctx, &project, r),
            Intent::SubmitReport(r) => self.submit_report(ctx, &project, r),
            Intent::UpdateFromMain(u) => self.update_from_main(ctx, &project, u),
            Intent::SuggestLink(l) => self.suggest_link(ctx, l),
            Intent::SubmitPlanDraft(d) => self.submit_plan_draft(ctx, d),
            Intent::ProposeSpec(s) => self.propose_spec(ctx, s),
            Intent::CreatePlan(p) => self.create_plan(ctx, p),
            Intent::RevisePlan(p) => self.revise_plan(ctx, p),
            Intent::ProposeRole(r) => self.propose_role(ctx, r),
            Intent::ProposeSoulChange(c) => self.needs_user(ctx, None, format!("change to SOUL.md proposed: {}", c.reason), serde_json::to_value(&c).ok()),
            Intent::ProposeRoleChange(c) => self.needs_user(ctx, None, format!("change to ROLE.md proposed: {}", c.reason), serde_json::to_value(&c).ok()),
            Intent::ProposeSkillUpdate(_) | Intent::ProposeNewSkill(_) | Intent::LoadSkill(_) => {
                Err(invalid("skills are handled by the agent runtime (load_skill, propose_skill_update, propose_new_skill during a run)"))
            }
        }?;
        for op in &out.ops {
            // Failures here are journaled and retried by reconcile; the intent itself is committed.
            if let Err(e) = self.run_op(*op) {
                tracing::warn!(op, error = %e, "journaled git operation failed; it is retried later");
            }
        }
        // An approval runs the merge right here: report its real result and the next step, not just "queued".
        if let (Some(key), false) = (reviewed, out.ops.is_empty()) {
            let t = self.ticket(ctx.project, &key)?;
            out.result["status"] = serde_json::json!(t.status);
            out.result["merged"] = serde_json::json!(t.merged);
            out.result["next"] = serde_json::json!(review_next(&t));
        }
        Ok(out)
    }

    fn commit<T: Serialize + serde::de::DeserializeOwned>(
        &self,
        ctx: &CallCtx,
        f: impl FnOnce(&mut WriteCtx<'_, '_>) -> Result<T>,
    ) -> Result<T> {
        Ok(self
            .store
            .commit(ctx.intent_id.as_deref(), &ctx.actor.label(), f)?
            .value)
    }

    fn needs_user(
        &self,
        ctx: &CallCtx,
        ticket: Option<TicketId>,
        reason: String,
        detail: Option<serde_json::Value>,
    ) -> Result<Outcome> {
        self.commit(ctx, |w| {
            w.emit(DomainEvent::NeedsUser { ticket, reason: reason.clone() })?;
            Ok(Outcome::ok(serde_json::json!({ "status": "waiting_for_user_approval", "reason": reason, "detail": detail })))
        })
    }
}
