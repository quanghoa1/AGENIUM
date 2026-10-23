//! The intent engine: validates a protocol call against the database (role, ownership, state machine, plan,
//! dependency DAG, scope) and commits its state changes, comments and events in one transaction, idempotent by
//! intent id. Git side effects are journaled in the same transaction and executed right after (see `effects`).

use std::collections::{HashMap, HashSet};

use agenium_core::comment::PermissionOutcome;
use agenium_core::forms::*;
use agenium_core::graph::{find_cycle, scopes_overlap};
use agenium_core::plan::PlanRevision;
use agenium_core::protocol::CreateAggregate;
use agenium_core::*;
use agenium_store::core::plans::{PlanRow, SpecRow};
use agenium_store::core::{agents, comments, plans, projects, runs, tickets, workspaces, WriteCtx};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::effects::Op;
use crate::{OrchError, Result};

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

fn denied(msg: impl Into<String>) -> OrchError {
    OrchError::Denied(msg.into())
}

fn invalid(msg: impl Into<String>) -> OrchError {
    OrchError::Invalid(msg.into())
}

fn ticket_by_key(w: &WriteCtx<'_, '_>, project: ProjectId, key: &TicketKey) -> Result<Ticket> {
    tickets::by_key(w.tx, project, key)?
        .ok_or_else(|| OrchError::NotFound(format!("ticket {key} does not exist in this project")))
}

/// The team of the project and whether `actor` is its Lead (Solo agent included). Users always may act as Lead.
fn require_lead(
    w: &WriteCtx<'_, '_>,
    project: ProjectId,
    actor: &Actor,
) -> Result<Option<agents::Team>> {
    let team = agents::teams_of_project(w.tx, project)?.into_iter().next();
    match actor {
        Actor::User => Ok(team),
        Actor::Lead(a) => {
            let t = team.ok_or_else(|| denied("the project has no team"))?;
            if t.lead == Some(*a) {
                Ok(Some(t))
            } else {
                Err(denied("only the team's Lead can do this"))
            }
        }
        _ => Err(denied("only the Lead or the user can do this")),
    }
}

/// The caller must be the assignee of `t` (worker tools).
fn require_assignee(t: &Ticket, actor: &Actor) -> Result<AgentId> {
    match actor {
        Actor::Worker(a) if t.assignee == Some(*a) => Ok(*a),
        Actor::Worker(_) => Err(denied(format!(
            "you are not the assignee of {}; workers act only on their own ticket",
            t.key
        ))),
        _ => Err(denied("this is a worker tool")),
    }
}

fn author_of(actor: &Actor) -> Author {
    match actor {
        Actor::Lead(a) | Actor::Worker(a) => Author::Agent(*a),
        Actor::User => Author::User,
        Actor::System => Author::System,
    }
}

fn add_comment(
    w: &mut WriteCtx<'_, '_>,
    t: &Ticket,
    author: Author,
    reply_to: Option<CommentId>,
    body: CommentBody,
) -> Result<Comment> {
    let c = comments::insert(
        w.tx,
        CommentId::new(),
        t.id,
        author,
        reply_to,
        &body,
        &now_iso(),
    )?;
    w.emit(DomainEvent::CommentAdded {
        comment: c.id,
        ticket: t.id,
        key: t.key.clone(),
        kind: c.kind,
    })?;
    Ok(c)
}

pub(crate) fn system_comment(
    w: &mut WriteCtx<'_, '_>,
    t: &Ticket,
    text: impl Into<String>,
) -> Result<Comment> {
    add_comment(
        w,
        t,
        Author::System,
        None,
        CommentBody::System { text: text.into() },
    )
}

fn set_agent_status(w: &mut WriteCtx<'_, '_>, agent: AgentId, to: AgentStatus) -> Result<()> {
    let from = agents::set_agent_status(w.tx, agent, to)?;
    if from != to {
        w.emit_as(
            "system",
            DomainEvent::AgentStatusChanged { agent, from, to },
        )?;
    }
    Ok(())
}

/// Checks and applies a status change, persisting the ticket and emitting events (escalation included).
pub(crate) fn transition(
    w: &mut WriteCtx<'_, '_>,
    t: &mut Ticket,
    actor: &Actor,
    to: TicketStatus,
    g: &Guards,
) -> Result<()> {
    check_transition(t, g, actor, to)?;
    let from = t.status;
    let fx = apply_transition(t, to);
    tickets::update(w.tx, t)?;
    w.emit_as(
        &actor.label(),
        DomainEvent::TicketStatusChanged {
            ticket: t.id,
            key: t.key.clone(),
            from,
            to,
        },
    )?;
    if matches!(
        to,
        TicketStatus::Resolved | TicketStatus::Closed | TicketStatus::Cancelled
    ) {
        runs::revoke_ticket_grants(w.tx, t.id, &now_iso())?;
    }
    if fx.escalated_now {
        let reason = format!(
            "{} was reopened {} times; the user decides how to continue",
            t.key, t.reopen_count
        );
        system_comment(w, t, &reason)?;
        w.emit_as(
            "system",
            DomainEvent::TicketEscalated {
                ticket: t.id,
                key: t.key.clone(),
                reason: reason.clone(),
            },
        )?;
        w.emit_as(
            "system",
            DomainEvent::NeedsUser {
                ticket: Some(t.id),
                reason,
            },
        )?;
    }
    Ok(())
}

/// Dependency satisfaction of a ticket, read from the database.
pub(crate) fn deps_satisfied(c: &rusqlite::Connection, t: &Ticket) -> Result<bool> {
    for k in &t.depends_on {
        let d = tickets::get_by_key(c, t.project, k)?;
        if !dep_satisfied(&d) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Every Question/Permission that blocks the ticket is resolved.
fn all_blocking_resolved(c: &rusqlite::Connection, t: &Ticket) -> Result<bool> {
    Ok(comments::open_blocking(c, t.id)?.is_empty())
}

/// Project allowlist = manifest network allowlist + user-granted always-allow entries.
pub(crate) fn allowlist(app: &App, p: &projects::Project) -> Vec<String> {
    let mut v = p.allowlist.clone();
    if let Ok(m) = app.manifest(p) {
        v.extend(m.network.allowlist);
    }
    v
}

fn domain_allowed(domain: &str, list: &[String]) -> bool {
    let d = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    list.iter().any(|e| {
        let e = e.trim().to_ascii_lowercase();
        if let Some(suffix) = e.strip_prefix("*.") {
            d.ends_with(&format!(".{suffix}")) || d == suffix
        } else {
            d == e
        }
    })
}

/// Whether a permission request is inside the project allowlist (then the Lead may approve it for the run).
pub fn in_allowlist(req: &PermissionRequest, list: &[String]) -> bool {
    match req.kind {
        PermissionKind::Network => {
            !req.domains.is_empty() && req.domains.iter().all(|d| domain_allowed(d, list))
        }
        PermissionKind::Path => {
            !req.paths.is_empty()
                && req
                    .paths
                    .iter()
                    .all(|p| list.iter().any(|e| e == &format!("path:{p}")))
        }
        PermissionKind::Command => req
            .command
            .as_ref()
            .is_some_and(|c| list.iter().any(|e| e == &format!("cmd:{c}"))),
    }
}

impl App {
    /// Applies a protocol intent. Errors are structured (see [`OrchError::to_tool_error`]) and change nothing.
    pub fn apply(&self, ctx: &CallCtx, intent: Intent) -> Result<Outcome> {
        let project = self.project(ctx.project)?;
        let out = match intent {
            Intent::CreateTicket(nt) => self.create_ticket(ctx, &project, nt),
            Intent::AssignTicket(a) => self.assign_ticket(ctx, a),
            Intent::AnswerQuestion(a) => self.answer_question(ctx, a),
            Intent::ReviewTicket(r) => self.review_ticket(ctx, &project, r),
            Intent::CloseTicket(c) => self.close_ticket(ctx, c),
            Intent::CancelTicket(c) => self.cancel_ticket(ctx, c),
            Intent::CreateHandoff(h) => self.create_handoff(ctx, h),
            Intent::ApprovePermission(a) => self.approve_permission(ctx, &project, a),
            Intent::GetTeam(_) => self.get_team(ctx),
            Intent::ConfirmLink(l) => self.confirm_link(ctx, l),
            Intent::CreateAggregate(a) => self.create_aggregate(ctx, &project, a),
            Intent::PostProgress(p) => self.post_progress(ctx, p),
            Intent::AskQuestion(q) => self.ask_question(ctx, q),
            Intent::RequestPermission(r) => self.request_permission(ctx, &project, r),
            Intent::SubmitReport(r) => self.submit_report(ctx, &project, r),
            Intent::SuggestLink(l) => self.suggest_link(ctx, l),
            Intent::ProposeSpec(s) => self.propose_spec(ctx, s),
            Intent::CreatePlan(p) => self.create_plan(ctx, p),
            Intent::RevisePlan(p) => self.revise_plan(ctx, p),
            Intent::ProposeRole(r) => self.needs_user(ctx, None, format!("Lead proposes a new role {:?}: {}", r.role_id, r.description), serde_json::to_value(&r).ok()),
            Intent::ProposeSoulChange(c) => self.needs_user(ctx, None, format!("change to SOUL.md proposed: {}", c.reason), serde_json::to_value(&c).ok()),
            Intent::ProposeRoleChange(c) => self.needs_user(ctx, None, format!("change to ROLE.md proposed: {}", c.reason), serde_json::to_value(&c).ok()),
            Intent::ProposeSkillUpdate(_) | Intent::LoadSkill(_) => {
                Err(invalid("skills are handled by the agent runtime; propose_skill_update is not available yet"))
            }
        }?;
        for op in &out.ops {
            // Failures here are journaled and retried by reconcile; the intent itself is committed.
            if let Err(e) = self.run_op(*op) {
                tracing::warn!(op, error = %e, "journaled git operation failed; reconcile will retry");
            }
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

    // ------------------------------------------------------------ spec & plan (decision 3h)

    fn propose_spec(
        &self,
        ctx: &CallCtx,
        spec: agenium_core::plan::RequirementSpec,
    ) -> Result<Outcome> {
        let project = ctx.project;
        self.commit(ctx, |w| {
            let team = require_lead(w, project, &ctx.actor)?;
            let version = plans::latest_spec(w.tx, project)?.map_or(1, |s| s.version + 1);
            // Solo self-approves (decision 6); a team's spec waits for the user.
            let solo = team.as_ref().is_some_and(|t| t.solo) || ctx.actor == Actor::User;
            let row = SpecRow {
                id: ulid::Ulid::new().to_string(),
                project,
                version,
                body: spec,
                proposed_by: ctx.actor.label(),
                approved_by: solo.then(|| ctx.actor.label()),
                locked_at: solo.then(now_iso),
            };
            plans::insert_spec(w.tx, &row)?;
            w.emit(DomainEvent::SpecProposed { project, version })?;
            if solo {
                w.emit(DomainEvent::SpecApproved {
                    project,
                    version,
                    by: ctx.actor.label(),
                })?;
            } else {
                w.emit(DomainEvent::NeedsUser {
                    ticket: None,
                    reason: format!("requirement spec v{version} waits for your approval"),
                })?;
            }
            Ok(Outcome::ok(
                serde_json::json!({ "spec_version": version, "locked": solo }),
            ))
        })
    }

    /// The user approves (locks) a spec version.
    pub fn approve_spec(&self, project: ProjectId, version: u32) -> Result<()> {
        self.store.commit(None, "user", |w| -> Result<()> {
            plans::approve_spec(w.tx, project, version, "user", &now_iso())?;
            w.emit(DomainEvent::SpecApproved {
                project,
                version,
                by: "user".into(),
            })?;
            Ok(())
        })?;
        Ok(())
    }

    fn create_plan(&self, ctx: &CallCtx, plan: agenium_core::plan::TicketPlan) -> Result<Outcome> {
        let project = ctx.project;
        self.commit(ctx, |w| {
            let team = require_lead(w, project, &ctx.actor)?;
            let spec = plans::spec(w.tx, project, plan.spec_version)?.ok_or_else(|| {
                invalid(format!(
                    "spec v{} does not exist; propose_spec first",
                    plan.spec_version
                ))
            })?;
            if spec.locked_at.is_none() {
                return Err(invalid(format!(
                    "spec v{} is not approved yet",
                    spec.version
                )));
            }
            check_trace(&spec, &plan.items)?;
            if plans::latest_plan(w.tx, project)?.is_some() {
                return Err(invalid("a plan already exists; use revise_plan"));
            }
            let solo = team.as_ref().is_some_and(|t| t.solo) || ctx.actor == Actor::User;
            let row = PlanRow {
                id: ulid::Ulid::new().to_string(),
                project,
                version: 1,
                spec_version: plan.spec_version,
                reason: None,
                diff: None,
                body: plan,
                proposed_by: ctx.actor.label(),
                approved_by: solo.then(|| ctx.actor.label()),
            };
            plans::insert_plan(w.tx, &row)?;
            w.emit(DomainEvent::PlanProposed {
                project,
                version: 1,
                needs_user: !solo,
            })?;
            if solo {
                w.emit(DomainEvent::PlanApproved {
                    project,
                    version: 1,
                    by: ctx.actor.label(),
                })?;
            } else {
                w.emit(DomainEvent::NeedsUser {
                    ticket: None,
                    reason: "plan v1 waits for your approval".into(),
                })?;
            }
            Ok(Outcome::ok(
                serde_json::json!({ "plan_version": 1, "approved": solo }),
            ))
        })
    }

    fn revise_plan(&self, ctx: &CallCtx, rev: PlanRevision) -> Result<Outcome> {
        let project = ctx.project;
        self.commit(ctx, |w| {
            let team = require_lead(w, project, &ctx.actor)?;
            let base = plans::latest_plan(w.tx, project)?.ok_or_else(|| invalid("no plan to revise; use create_plan"))?;
            if base.version != rev.base_version {
                return Err(OrchError::Conflict(format!("base_version {} is stale; the latest plan is v{}", rev.base_version, base.version)));
            }
            let spec = plans::spec(w.tx, project, base.body.spec_version)?.ok_or_else(|| invalid("plan spec missing"))?;
            check_trace(&spec, &rev.items)?;
            // Items that have tickets must keep existing (shrink scope, never drop work silently).
            for t in tickets::list(w.tx, project, &Default::default())? {
                if !t.status.is_terminal() && !rev.items.iter().any(|i| i.id == t.plan_item) {
                    return Err(invalid(format!("plan item {} still has open ticket {}", t.plan_item, t.key)));
                }
            }
            let (diff, scope_change) = plan_diff(&base.body.items, &rev.items);
            let solo = team.as_ref().is_some_and(|t| t.solo) || ctx.actor == Actor::User;
            // Minor revisions: the Lead approves itself and the user is informed; scope changes need the user.
            let self_approve = ctx.actor == Actor::User || !scope_change;
            let version = base.version + 1;
            let row = PlanRow {
                id: ulid::Ulid::new().to_string(),
                project,
                version,
                spec_version: base.body.spec_version,
                reason: Some(rev.reason.clone()),
                diff: Some(diff.clone()),
                body: agenium_core::plan::TicketPlan { spec_version: base.body.spec_version, milestones: rev.milestones, items: rev.items },
                proposed_by: ctx.actor.label(),
                approved_by: self_approve.then(|| ctx.actor.label()),
            };
            plans::insert_plan(w.tx, &row)?;
            w.emit(DomainEvent::PlanProposed { project, version, needs_user: !self_approve })?;
            if self_approve {
                w.emit(DomainEvent::PlanApproved { project, version, by: ctx.actor.label() })?;
            } else {
                w.emit(DomainEvent::NeedsUser { ticket: None, reason: format!("plan v{version} changes scope and waits for your approval: {}", rev.reason) })?;
            }
            let _ = solo;
            Ok(Outcome::ok(serde_json::json!({ "plan_version": version, "approved": self_approve, "scope_change": scope_change, "diff": diff })))
        })
    }

    /// The user approves a pending plan version.
    pub fn approve_plan(&self, project: ProjectId, version: u32) -> Result<()> {
        self.store.commit(None, "user", |w| -> Result<()> {
            plans::approve_plan(w.tx, project, version, "user")?;
            w.emit(DomainEvent::PlanApproved {
                project,
                version,
                by: "user".into(),
            })?;
            Ok(())
        })?;
        Ok(())
    }

    // ------------------------------------------------------------ tickets

    fn create_ticket(
        &self,
        ctx: &CallCtx,
        project: &projects::Project,
        nt: NewTicket,
    ) -> Result<Outcome> {
        let pid = project.id;
        let default_template = self
            .manifest(project)
            .map(|m| m.project.template)
            .unwrap_or_else(|_| "coding@1".into());
        self.commit(ctx, |w| {
            require_lead(w, pid, &ctx.actor)?;
            let plan = plans::current_plan(w.tx, pid)?.ok_or_else(|| invalid("no approved plan: every ticket must reference a plan item (create_plan first)"))?;
            if !plan.body.items.iter().any(|i| i.id == nt.plan_item) {
                return Err(invalid(format!("plan item {} is not in the approved plan v{}", nt.plan_item, plan.version)));
            }
            let parent = match &nt.parent {
                Some(k) => Some(ticket_by_key(w, pid, k)?),
                None => None,
            };
            let mut dep_ids = Vec::new();
            for k in &nt.depends_on {
                dep_ids.push(ticket_by_key(w, pid, k)?.id);
            }
            // Overlapping scope with an open ticket requires a dependency path between them (decision 18).
            if !nt.scope_paths.is_empty() {
                let graph = tickets::dep_graph(w.tx, pid)?;
                let ancestors = closure(&graph, &dep_ids);
                for other in tickets::list(w.tx, pid, &Default::default())? {
                    if !other.status.is_terminal() && !ancestors.contains(&other.id) && scopes_overlap(&nt.scope_paths, &other.scope_paths) {
                        return Err(invalid(format!(
                            "scope_paths overlap with open ticket {} ({:?}); add it to depends_on or narrow the scope",
                            other.key, other.scope_paths
                        )));
                    }
                }
            }
            let key = projects::next_ticket_key(w.tx, pid)?;
            let id = TicketId::new();
            // A new ticket has no dependents yet, so it cannot close a cycle; checked anyway for safety.
            let graph = tickets::dep_graph(w.tx, pid)?;
            if let Some(cyc) = find_cycle(&graph, &id, &dep_ids) {
                return Err(invalid(format!("depends_on would create a cycle ({} tickets)", cyc.len())));
            }
            let t = tickets::insert(
                w.tx,
                &tickets::TicketInsert {
                    id,
                    key: key.clone(),
                    project: pid,
                    parent: parent.map(|p| p.id),
                    kind: nt.kind,
                    title: nt.title.trim().to_owned(),
                    description: nt.description,
                    acceptance: nt.acceptance,
                    plan_item: nt.plan_item,
                    priority: nt.priority,
                    depends_on: dep_ids,
                    required_role: nt.required_role,
                    required_tags: nt.required_tags,
                    scope_paths: nt.scope_paths,
                    report_template: nt.report_template.unwrap_or(default_template),
                    created_at: now_iso(),
                },
            )?;
            w.emit(DomainEvent::TicketCreated { ticket: t.id, key: t.key.clone(), title: t.title.clone() })?;
            Ok(Outcome::ok(serde_json::json!({ "ticket_id": t.key, "status": t.status })))
        })
    }

    fn assign_ticket(&self, ctx: &CallCtx, a: AssignTicket) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            let team = require_lead(w, pid, &ctx.actor)?.ok_or_else(|| invalid("the project has no team"))?;
            let mut t = ticket_by_key(w, pid, &a.ticket_id)?;
            let agent = agents::find_agent(w.tx, a.worker)?.ok_or_else(|| invalid(format!("unknown agent {}", a.worker)))?;
            let in_team = agent.team == team.id && (agent.role_kind == RoleKind::Worker || team.solo);
            if !in_team {
                return Err(invalid(format!("{} is not a worker of this team", agent.name)));
            }
            if !agent.status.assignable() {
                return Err(invalid(format!("{} is {} and cannot take work", agent.name, agent.status)));
            }
            if let Some(role) = &t.required_role {
                if &agent.role != role && !team.solo {
                    return Err(invalid(format!("{} requires role {role}, {} is {}", t.key, agent.name, agent.role)));
                }
            }
            let missing: Vec<&String> = t.required_tags.iter().filter(|tag| !agent.tags.contains(tag)).collect();
            if !missing.is_empty() && !team.solo {
                return Err(invalid(format!("{} lacks required tags {missing:?}", agent.name)));
            }
            let has_plan_item = plans::current_plan(w.tx, pid)?.is_some_and(|p| p.body.items.iter().any(|i| i.id == t.plan_item));
            let mut ops = vec![];
            match t.status {
                TicketStatus::New => {
                    t.assignee = Some(agent.id);
                    let g = Guards { assignee_in_team: true, has_plan_item, ..Default::default() };
                    transition(w, &mut t, &ctx.actor, TicketStatus::Assigned, &g)?;
                    w.emit(DomainEvent::TicketAssigned { ticket: t.id, key: t.key.clone(), agent: agent.id })?;
                    ops.push(crate::effects::journal(w, &t, Op::CreateWorktree)?);
                }
                TicketStatus::Reopened => {
                    // Reassignment of a reopened ticket keeps its status and worktree.
                    t.assignee = Some(agent.id);
                    tickets::update(w.tx, &t)?;
                    w.emit(DomainEvent::TicketAssigned { ticket: t.id, key: t.key.clone(), agent: agent.id })?;
                }
                s => return Err(invalid(format!("{} is {s}; only New or Reopened tickets can be assigned", t.key))),
            }
            if let Some(note) = a.note.filter(|n| !n.trim().is_empty()) {
                add_comment(w, &t, author_of(&ctx.actor), None, CommentBody::Text { text: note })?;
            }
            Ok(Outcome { result: serde_json::json!({ "ticket_id": t.key, "status": t.status, "assignee": agent.id }), run: RunEffect::Continue, ops })
        })
    }

    fn post_progress(&self, ctx: &CallCtx, p: ProgressUpdate) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            let t = ticket_by_key(w, pid, &p.ticket_id)?;
            require_assignee(&t, &ctx.actor)?;
            let c = add_comment(w, &t, author_of(&ctx.actor), None, CommentBody::Progress(p))?;
            Ok(Outcome::ok(serde_json::json!({ "comment_id": c.id })))
        })
    }

    fn ask_question(&self, ctx: &CallCtx, q: Question) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            let mut t = ticket_by_key(w, pid, &q.ticket_id)?;
            let agent = require_assignee(&t, &ctx.actor)?;
            if t.status != TicketStatus::InProgress && t.status != TicketStatus::Pending {
                return Err(invalid(format!("{} is {}; questions are asked while working on it", t.key, t.status)));
            }
            let blocking = q.blocking;
            let c = add_comment(w, &t, Author::Agent(agent), None, CommentBody::Question(q))?;
            let solo = agents::get_team(w.tx, agents::get_agent(w.tx, agent)?.team)?.solo;
            if blocking {
                if t.status == TicketStatus::InProgress {
                    transition(w, &mut t, &ctx.actor, TicketStatus::Pending, &Guards { open_blocking: true, ..Default::default() })?;
                }
                set_agent_status(w, agent, AgentStatus::Waiting)?;
                if solo {
                    // A Solo agent's questions go to the user (it is its own Lead).
                    w.emit_as("system", DomainEvent::NeedsUser { ticket: Some(t.id), reason: format!("{} asks a blocking question", t.key) })?;
                }
            }
            Ok(Outcome {
                result: serde_json::json!({ "comment_id": c.id, "blocking": blocking, "routed_to": if solo { "user" } else { "lead" } }),
                run: if blocking { RunEffect::Park } else { RunEffect::Continue },
                ops: vec![],
            })
        })
    }

    fn answer_question(&self, ctx: &CallCtx, a: AnswerQuestion) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            require_lead(w, pid, &ctx.actor)?;
            let mut t = ticket_by_key(w, pid, &a.ticket_id)?;
            let q = comments::find(w.tx, a.reply_to)?
                .ok_or_else(|| invalid(format!("comment {} does not exist", a.reply_to)))?;
            if q.ticket != t.id || q.kind != CommentKind::Question {
                return Err(invalid("reply_to must be a Question on this ticket"));
            }
            if q.resolved {
                return Err(invalid("this question is already answered"));
            }
            if let (Some(i), CommentBody::Question(qq)) = (a.chosen_option, &q.body) {
                if i >= qq.options.len() {
                    return Err(invalid(
                        "chosen_option is not an index into the question's options",
                    ));
                }
            }
            let ans = add_comment(
                w,
                &t,
                author_of(&ctx.actor),
                Some(q.id),
                CommentBody::Answer(Answer {
                    resolution: a.answer,
                    chosen_option: a.chosen_option,
                }),
            )?;
            comments::set_resolved(w.tx, q.id, true)?;
            w.emit(DomainEvent::CommentResolved {
                comment: q.id,
                ticket: t.id,
            })?;
            unblock_if_clear(w, &mut t)?;
            Ok(Outcome::ok(
                serde_json::json!({ "comment_id": ans.id, "ticket_status": t.status }),
            ))
        })
    }

    fn request_permission(
        &self,
        ctx: &CallCtx,
        project: &projects::Project,
        r: PermissionRequest,
    ) -> Result<Outcome> {
        let pid = ctx.project;
        let inside = in_allowlist(&r, &allowlist(self, project));
        self.commit(ctx, |w| {
            let mut t = ticket_by_key(w, pid, &r.ticket_id)?;
            let agent = require_assignee(&t, &ctx.actor)?;
            if t.status != TicketStatus::InProgress && t.status != TicketStatus::Pending {
                return Err(invalid(format!("{} is {}; permissions are requested while working on it", t.key, t.status)));
            }
            let c = add_comment(w, &t, Author::Agent(agent), None, CommentBody::Permission { request: r, run: ctx.run, outcome: None })?;
            if t.status == TicketStatus::InProgress {
                transition(w, &mut t, &ctx.actor, TicketStatus::Pending, &Guards { open_blocking: true, ..Default::default() })?;
            }
            set_agent_status(w, agent, AgentStatus::Waiting)?;
            let solo = agents::get_team(w.tx, agents::get_agent(w.tx, agent)?.team)?.solo;
            let decider = if inside && !solo { "lead" } else { "user" };
            if decider == "user" {
                w.emit_as("system", DomainEvent::NeedsUser { ticket: Some(t.id), reason: format!("{} requests a permission outside the allowlist", t.key) })?;
            }
            Ok(Outcome { result: serde_json::json!({ "request_id": c.id, "in_allowlist": inside, "decided_by": decider }), run: RunEffect::Park, ops: vec![] })
        })
    }

    fn approve_permission(
        &self,
        ctx: &CallCtx,
        project: &projects::Project,
        a: ApprovePermission,
    ) -> Result<Outcome> {
        if ctx.actor == Actor::User {
            return self.decide_permission(
                project.id,
                &a.ticket_id,
                a.request_id,
                a.decision,
                false,
                a.note,
            );
        }
        let list = allowlist(self, project);
        let pid = ctx.project;
        self.commit(ctx, |w| {
            require_lead(w, pid, &ctx.actor)?;
            let mut t = ticket_by_key(w, pid, &a.ticket_id)?;
            let c = comments::find(w.tx, a.request_id)?.ok_or_else(|| invalid("request_id does not exist"))?;
            let CommentBody::Permission { request, run, outcome: None } = c.body.clone() else {
                return Err(invalid("request_id is not an undecided permission request"));
            };
            if c.ticket != t.id {
                return Err(invalid("the request belongs to another ticket"));
            }
            if a.decision == PermissionDecision::Approve && !in_allowlist(&request, &list) {
                return Err(denied("the request is outside the project allowlist: only the user can approve it"));
            }
            record_permission(w, &mut t, &c, request, run, a.decision, &ctx.actor.label(), "run", a.note)?;
            Ok(Outcome::ok(serde_json::json!({ "decision": a.decision, "scope": "run", "ticket_status": t.status })))
        })
    }

    /// The user decides a permission request; `always` adds the request to the project allowlist (only the
    /// user can grant always-allow, decision 4).
    pub fn decide_permission(
        &self,
        project: ProjectId,
        key: &TicketKey,
        request_id: CommentId,
        decision: PermissionDecision,
        always: bool,
        note: Option<String>,
    ) -> Result<Outcome> {
        self.store
            .commit(None, "user", |w| -> Result<Outcome> {
                let mut t = ticket_by_key(w, project, key)?;
                let c = comments::find(w.tx, request_id)?.ok_or_else(|| invalid("request_id does not exist"))?;
                let CommentBody::Permission { request, run, outcome: None } = c.body.clone() else {
                    return Err(invalid("request_id is not an undecided permission request"));
                };
                if c.ticket != t.id {
                    return Err(invalid("the request belongs to another ticket"));
                }
                let scope = if always && decision == PermissionDecision::Approve { "project" } else { "run" };
                if scope == "project" {
                    let entries: Vec<String> = match request.kind {
                        PermissionKind::Network => request.domains.clone(),
                        PermissionKind::Path => request.paths.iter().map(|p| format!("path:{p}")).collect(),
                        PermissionKind::Command => request.command.iter().map(|c| format!("cmd:{c}")).collect(),
                    };
                    projects::add_allowlist(w.tx, project, &entries)?;
                }
                record_permission(w, &mut t, &c, request, run, decision, "user", scope, note)?;
                Ok(Outcome::ok(serde_json::json!({ "decision": decision, "scope": scope, "ticket_status": t.status })))
            })
            .map(|c| c.value)
    }

    /// The user answers an open Question (Solo agents, or escalations).
    pub fn user_answer(
        &self,
        project: ProjectId,
        key: &TicketKey,
        question: CommentId,
        answer: &str,
        chosen_option: Option<usize>,
    ) -> Result<Outcome> {
        let ctx = CallCtx::user(project);
        self.answer_question(
            &ctx,
            AnswerQuestion {
                ticket_id: key.clone(),
                reply_to: question,
                answer: answer.to_owned(),
                chosen_option,
            },
        )
    }

    /// A free-text `User` comment on a ticket (steering, decision 19).
    pub fn user_comment(&self, project: ProjectId, key: &TicketKey, text: &str) -> Result<Comment> {
        self.store
            .commit(None, "user", |w| -> Result<Comment> {
                let t = ticket_by_key(w, project, key)?;
                add_comment(
                    w,
                    &t,
                    Author::User,
                    None,
                    CommentBody::Text {
                        text: text.to_owned(),
                    },
                )
            })
            .map(|c| c.value)
    }

    fn submit_report(
        &self,
        ctx: &CallCtx,
        project: &projects::Project,
        r: WorkReport,
    ) -> Result<Outcome> {
        let pid = ctx.project;
        // Pre-checks outside the transaction so the auto-commit only happens for a report that can be accepted.
        let t0 = self
            .store
            .read(|c| tickets::get_by_key(c, pid, &r.ticket_id))?;
        let agent = require_assignee(&t0, &ctx.actor)?;
        if t0.status != TicketStatus::InProgress {
            return Err(invalid(format!(
                "{} is {}; submit_report is only valid while InProgress",
                t0.key, t0.status
            )));
        }
        check_report_covers_acceptance(&t0, &r)?;
        crate::templates::validate_extension(&t0.report_template, &r.extension)?;
        // Auto-commit the worktree (journal-first) and learn whether the ticket carries code.
        let commit = self.auto_commit(project, &t0, agent, ctx.intent_id.as_deref())?;
        self.commit(ctx, |w| {
            let mut t = tickets::get(w.tx, t0.id)?;
            if t.status != TicketStatus::InProgress {
                return Err(invalid(format!("{} changed to {} meanwhile", t.key, t.status)));
            }
            t.has_code = commit.has_code;
            let c = add_comment(w, &t, Author::Agent(agent), None, CommentBody::Report(r))?;
            transition(w, &mut t, &ctx.actor, TicketStatus::Resolved, &Guards { valid_report: true, ..Default::default() })?;
            set_agent_status(w, agent, AgentStatus::Idle)?;
            if let Some(ws) = workspaces::get(w.tx, t.id)? {
                workspaces::upsert(w.tx, &workspaces::TicketWorkspace { head_sha: commit.head.clone(), ..ws })?;
            }
            Ok(Outcome {
                result: serde_json::json!({ "ticket_id": t.key, "status": t.status, "report_comment": c.id, "has_code": t.has_code, "commit": commit.sha }),
                run: RunEffect::End,
                ops: vec![],
            })
        })
    }

    fn review_ticket(
        &self,
        ctx: &CallCtx,
        project: &projects::Project,
        r: ReviewDecision,
    ) -> Result<Outcome> {
        let pid = ctx.project;
        let review_gate = project.review_before_merge();
        self.commit(ctx, |w| {
            let team = require_lead(w, pid, &ctx.actor)?;
            let mut t = ticket_by_key(w, pid, &r.ticket_id)?;
            if t.status != TicketStatus::Resolved {
                return Err(invalid(format!("{} is {}; only Resolved tickets are reviewed", t.key, t.status)));
            }
            let ac_ids: HashSet<&str> = t.acceptance.iter().map(|a| a.id.as_str()).collect();
            for f in &r.ac_feedback {
                if !ac_ids.contains(f.ac_id.as_str()) {
                    return Err(invalid(format!("{} has no acceptance criterion {}", t.key, f.ac_id)));
                }
            }
            let decision = r.decision;
            add_comment(w, &t, author_of(&ctx.actor), None, CommentBody::Review(r))?;
            let mut ops = vec![];
            match decision {
                ReviewVerdict::Approve => {
                    if t.approved {
                        return Err(invalid(format!("{} is already approved", t.key)));
                    }
                    t.approved = true;
                    tickets::update(w.tx, &t)?;
                    w.emit(DomainEvent::TicketApproved { ticket: t.id, key: t.key.clone() })?;
                    let solo_gate = review_gate && team.as_ref().is_some_and(|t| t.solo) && ctx.actor != Actor::User;
                    if t.has_code && !t.merged {
                        if solo_gate {
                            // Project option "review before merge" (decision 28): the user approves the merge.
                            w.emit_as("system", DomainEvent::NeedsUser { ticket: Some(t.id), reason: format!("{} is approved by the agent and waits for your review before merge", t.key) })?;
                        } else {
                            ops.push(crate::effects::journal(w, &t, Op::Merge)?);
                        }
                    }
                    // Dependents may be unblocked now (approved, and merged when it has code).
                }
                ReviewVerdict::Reopen => {
                    transition(w, &mut t, &ctx.actor, TicketStatus::Reopened, &Guards { per_ac_feedback: true, ..Default::default() })?;
                }
            }
            Ok(Outcome { result: serde_json::json!({ "ticket_id": t.key, "status": t.status, "approved": t.approved, "merge_queued": !ops.is_empty() }), run: RunEffect::Continue, ops })
        })
    }

    /// The user releases a merge held by "review before merge".
    pub fn user_approve_merge(&self, project: ProjectId, key: &TicketKey) -> Result<Outcome> {
        let out = self.store.commit(None, "user", |w| -> Result<Outcome> {
            let t = ticket_by_key(w, project, key)?;
            if !(t.status == TicketStatus::Resolved && t.approved && t.has_code && !t.merged) {
                return Err(invalid(format!("{} is not waiting for a merge", t.key)));
            }
            let op = crate::effects::journal(w, &t, Op::Merge)?;
            Ok(Outcome {
                result: serde_json::json!({ "ticket_id": t.key, "merge_queued": true }),
                run: RunEffect::Continue,
                ops: vec![op],
            })
        })?;
        for op in &out.value.ops {
            if let Err(e) = self.run_op(*op) {
                tracing::warn!(op, error = %e, "merge failed; reconcile will retry");
            }
        }
        Ok(out.value)
    }

    fn close_ticket(&self, ctx: &CallCtx, c: CloseTicket) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            require_lead(w, pid, &ctx.actor)?;
            let mut t = ticket_by_key(w, pid, &c.ticket_id)?;
            if t.status == TicketStatus::Resolved && !t.approved {
                return Err(invalid(format!(
                    "{} is not approved yet: review_ticket first",
                    t.key
                )));
            }
            t.close_reason = Some(c.reason.trim().to_owned());
            transition(
                w,
                &mut t,
                &ctx.actor,
                TicketStatus::Closed,
                &Guards {
                    close_reason: true,
                    ..Default::default()
                },
            )?;
            let mut ops = vec![];
            if workspaces::get(w.tx, t.id)?.is_some_and(|ws| !ws.removed) {
                ops.push(crate::effects::journal(
                    w,
                    &t,
                    Op::RemoveWorktree {
                        delete_branch: t.merged,
                    },
                )?);
            }
            plan_item_done(w, &t)?;
            Ok(Outcome {
                result: serde_json::json!({ "ticket_id": t.key, "status": t.status }),
                run: RunEffect::Continue,
                ops,
            })
        })
    }

    fn cancel_ticket(&self, ctx: &CallCtx, c: CancelTicket) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            require_lead(w, pid, &ctx.actor)?;
            let mut t = ticket_by_key(w, pid, &c.ticket_id)?;
            let was = t.status;
            t.close_reason = Some(c.reason.trim().to_owned());
            transition(
                w,
                &mut t,
                &ctx.actor,
                TicketStatus::Cancelled,
                &Guards::default(),
            )?;
            system_comment(w, &t, format!("Cancelled: {}", c.reason.trim()))?;
            if let Some(a) = t.assignee {
                if matches!(was, TicketStatus::InProgress | TicketStatus::Pending) {
                    set_agent_status(w, a, AgentStatus::Idle)?;
                }
            }
            let mut ops = vec![];
            if workspaces::get(w.tx, t.id)?.is_some_and(|ws| !ws.removed) {
                // Keep the unmerged branch (decision 11: delete the worktree, keep branches that were not merged).
                ops.push(crate::effects::journal(
                    w,
                    &t,
                    Op::RemoveWorktree {
                        delete_branch: false,
                    },
                )?);
            }
            Ok(Outcome {
                result: serde_json::json!({ "ticket_id": t.key, "status": t.status }),
                run: RunEffect::Continue,
                ops,
            })
        })
    }

    fn create_handoff(&self, ctx: &CallCtx, h: Handoff) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            require_lead(w, pid, &ctx.actor)?;
            let src = ticket_by_key(w, pid, &h.source_ticket)?;
            let dst = ticket_by_key(w, pid, &h.target_ticket)?;
            if dst.status.is_terminal() {
                return Err(invalid(format!("{} is {}", dst.key, dst.status)));
            }
            let c = add_comment(
                w,
                &dst,
                author_of(&ctx.actor),
                None,
                CommentBody::Handoff(h),
            )?;
            tickets::add_link(
                w.tx,
                src.id,
                dst.id,
                "related_to",
                &ctx.actor.label(),
                Some(&ctx.actor.label()),
                Some("handoff"),
            )?;
            w.emit(DomainEvent::LinkAdded {
                ticket: dst.id,
                from: src.key.clone(),
                to: dst.key.clone(),
                kind: "related_to".into(),
                confirmed: true,
            })?;
            Ok(Outcome::ok(serde_json::json!({ "comment_id": c.id })))
        })
    }

    fn suggest_link(&self, ctx: &CallCtx, l: SuggestLink) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            let from = ticket_by_key(w, pid, &l.from)?;
            require_assignee(&from, &ctx.actor)?;
            let to = ticket_by_key(w, pid, &l.to)?;
            tickets::add_link(
                w.tx,
                from.id,
                to.id,
                "related_to",
                &ctx.actor.label(),
                None,
                Some(&l.reason),
            )?;
            w.emit(DomainEvent::LinkAdded {
                ticket: from.id,
                from: from.key.clone(),
                to: to.key.clone(),
                kind: "related_to".into(),
                confirmed: false,
            })?;
            Ok(Outcome::ok(
                serde_json::json!({ "proposed": true, "awaiting": "lead confirm_link" }),
            ))
        })
    }

    fn confirm_link(&self, ctx: &CallCtx, l: ConfirmLink) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            require_lead(w, pid, &ctx.actor)?;
            let from = ticket_by_key(w, pid, &l.from)?;
            let to = ticket_by_key(w, pid, &l.to)?;
            if l.kind == LinkKind::Blocks {
                return Err(invalid(
                    "blocks links are derived from depends_on; change the dependency instead",
                ));
            }
            tickets::add_link(
                w.tx,
                from.id,
                to.id,
                l.kind.as_str(),
                &ctx.actor.label(),
                Some(&ctx.actor.label()),
                None,
            )?;
            w.emit(DomainEvent::LinkAdded {
                ticket: from.id,
                from: from.key.clone(),
                to: to.key.clone(),
                kind: l.kind.as_str().into(),
                confirmed: true,
            })?;
            Ok(Outcome::ok(serde_json::json!({ "confirmed": true })))
        })
    }

    fn create_aggregate(
        &self,
        ctx: &CallCtx,
        project: &projects::Project,
        a: CreateAggregate,
    ) -> Result<Outcome> {
        // An aggregate is an ordinary ticket on the plan item of its first child, linked `aggregates` to each child.
        let first = self
            .store
            .read(|c| tickets::get_by_key(c, project.id, &a.children[0]))?;
        let nt = NewTicket {
            kind: TicketKind::Task,
            plan_item: first.plan_item.clone(),
            parent: None,
            title: a.title.clone(),
            description: format!(
                "{}\n\nAggregates: {}",
                a.summary,
                a.children
                    .iter()
                    .map(|k| k.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            acceptance: vec![AcceptanceCriterion {
                id: "AC-1".into(),
                text: format!("{} verified end to end", a.kind),
            }],
            priority: first.priority,
            depends_on: vec![],
            required_role: None,
            required_tags: vec![],
            scope_paths: vec![],
            report_template: None,
        };
        let created = self.create_ticket(
            &CallCtx {
                intent_id: ctx.intent_id.as_ref().map(|i| format!("{i}:ticket")),
                ..ctx.clone()
            },
            project,
            nt,
        )?;
        let key: TicketKey = serde_json::from_value(created.result["ticket_id"].clone())
            .map_err(|e| invalid(e.to_string()))?;
        let pid = project.id;
        self.commit(ctx, |w| {
            let agg = ticket_by_key(w, pid, &key)?;
            for k in &a.children {
                let child = ticket_by_key(w, pid, k)?;
                tickets::add_link(
                    w.tx,
                    agg.id,
                    child.id,
                    "aggregates",
                    &ctx.actor.label(),
                    Some(&ctx.actor.label()),
                    Some(&a.kind),
                )?;
            }
            Ok(Outcome::ok(serde_json::json!({ "ticket_id": key })))
        })
    }

    fn get_team(&self, ctx: &CallCtx) -> Result<Outcome> {
        let (team, members) = self
            .team_of_project(ctx.project)?
            .ok_or_else(|| invalid("the project has no team"))?;
        if !matches!(ctx.actor, Actor::Lead(a) if team.lead == Some(a)) && ctx.actor != Actor::User
        {
            return Err(denied("only the Lead sees the team roster"));
        }
        let open: HashMap<AgentId, TicketKey> = self
            .tickets(ctx.project)?
            .into_iter()
            .filter(|t| matches!(t.status, TicketStatus::InProgress | TicketStatus::Pending))
            .filter_map(|t| t.assignee.map(|a| (a, t.key)))
            .collect();
        let roster: Vec<serde_json::Value> = members
            .iter()
            .map(|m| {
                serde_json::json!({
                    "agent": m.id, "name": m.name, "role": m.role, "kind": m.role_kind, "tags": m.tags,
                    "model": format!("{}/{}", m.model.provider, m.model.model), "status": m.status,
                    "current_ticket": open.get(&m.id),
                })
            })
            .collect();
        Ok(Outcome::ok(
            serde_json::json!({ "team": team.name, "solo": team.solo, "members": roster }),
        ))
    }
}

fn closure(graph: &HashMap<TicketId, Vec<TicketId>>, start: &[TicketId]) -> HashSet<TicketId> {
    let mut seen = HashSet::new();
    let mut stack: Vec<TicketId> = start.to_vec();
    while let Some(n) = stack.pop() {
        if seen.insert(n) {
            stack.extend(graph.get(&n).into_iter().flatten().copied());
        }
    }
    seen
}

fn check_trace(spec: &SpecRow, items: &[agenium_core::plan::PlanItem]) -> Result<()> {
    let reqs: HashSet<&str> = spec
        .body
        .requirements
        .iter()
        .map(|r| r.id.as_str())
        .collect();
    for it in items {
        for t in &it.trace_to {
            if !reqs.contains(t.as_str()) {
                return Err(invalid(format!(
                    "plan item {} traces to {t}, which is not in spec v{}",
                    it.id, spec.version
                )));
            }
        }
    }
    Ok(())
}

/// Human-readable diff between plan versions and whether it changes scope (requirements covered or acceptance
/// criteria of existing items).
fn plan_diff(
    old: &[agenium_core::plan::PlanItem],
    new: &[agenium_core::plan::PlanItem],
) -> (String, bool) {
    let mut lines = Vec::new();
    let mut scope = false;
    let old_map: HashMap<&str, &agenium_core::plan::PlanItem> =
        old.iter().map(|i| (i.id.as_str(), i)).collect();
    let new_ids: HashSet<&str> = new.iter().map(|i| i.id.as_str()).collect();
    for it in new {
        match old_map.get(it.id.as_str()) {
            None => lines.push(format!("+ {} {}", it.id, it.title)),
            Some(o) => {
                if o.acceptance != it.acceptance {
                    lines.push(format!("~ {} acceptance changed", it.id));
                    scope = true;
                }
                if o.title != it.title
                    || o.depends_on != it.depends_on
                    || o.milestone != it.milestone
                {
                    lines.push(format!("~ {} {}", it.id, it.title));
                }
            }
        }
    }
    for o in old {
        if !new_ids.contains(o.id.as_str()) {
            lines.push(format!("- {} {}", o.id, o.title));
            scope = true;
        }
    }
    let covered = |v: &[agenium_core::plan::PlanItem]| {
        v.iter()
            .flat_map(|i| i.trace_to.iter().cloned())
            .collect::<HashSet<String>>()
    };
    if covered(old) != covered(new) {
        lines.push("~ requirement coverage changed".into());
        scope = true;
    }
    (lines.join("\n"), scope)
}

fn check_report_covers_acceptance(t: &Ticket, r: &WorkReport) -> Result<()> {
    let have: HashSet<&str> = r.acceptance.iter().map(|a| a.ac_id.as_str()).collect();
    let want: HashSet<&str> = t.acceptance.iter().map(|a| a.id.as_str()).collect();
    let missing: Vec<&&str> = want.difference(&have).collect();
    let unknown: Vec<&&str> = have.difference(&want).collect();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(invalid(format!(
            "the report must give one result per acceptance criterion of {}: missing {missing:?}, unknown {unknown:?}",
            t.key
        )));
    }
    Ok(())
}

/// After a blocking item is resolved: Pending -> InProgress (System) when nothing else blocks, agent -> Idle so the
/// scheduler starts a continuation run.
fn unblock_if_clear(w: &mut WriteCtx<'_, '_>, t: &mut Ticket) -> Result<()> {
    if t.status == TicketStatus::Pending && all_blocking_resolved(w.tx, t)? {
        transition(
            w,
            t,
            &Actor::System,
            TicketStatus::InProgress,
            &Guards {
                all_blocking_resolved: true,
                ..Default::default()
            },
        )?;
        if let Some(a) = t.assignee {
            set_agent_status(w, a, AgentStatus::Idle)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn record_permission(
    w: &mut WriteCtx<'_, '_>,
    t: &mut Ticket,
    c: &Comment,
    request: PermissionRequest,
    run: Option<RunId>,
    decision: PermissionDecision,
    by: &str,
    scope: &str,
    note: Option<String>,
) -> Result<()> {
    let body = CommentBody::Permission {
        request: request.clone(),
        run,
        outcome: Some(PermissionOutcome {
            decision,
            decided_by: by.to_owned(),
            scope: scope.to_owned(),
            note,
        }),
    };
    comments::set_body(w.tx, c.id, &body)?;
    comments::set_resolved(w.tx, c.id, true)?;
    w.emit(DomainEvent::CommentResolved {
        comment: c.id,
        ticket: t.id,
    })?;
    match decision {
        PermissionDecision::Approve => {
            runs::insert_grant(
                w.tx,
                &runs::Grant {
                    id: ulid::Ulid::new().to_string(),
                    project: t.project,
                    ticket: Some(t.id),
                    run: Some(run.unwrap_or_default_run()),
                    scope: scope.to_owned(),
                    granted_by: by.to_owned(),
                    request: serde_json::to_value(&request).unwrap_or_default(),
                },
            )?;
            w.emit(DomainEvent::PermissionGranted {
                ticket: t.id,
                request: c.id,
                scope: scope.to_owned(),
            })?;
        }
        PermissionDecision::Deny => {
            w.emit(DomainEvent::PermissionDenied {
                ticket: t.id,
                request: c.id,
            })?;
        }
    }
    unblock_if_clear(w, t)
}

trait RunOrDefault {
    fn unwrap_or_default_run(self) -> RunId;
}
impl RunOrDefault for Option<RunId> {
    /// User-created requests (no run) still need a run id for the audit trigger: a fresh marker id.
    fn unwrap_or_default_run(self) -> RunId {
        self.unwrap_or_else(RunId::new)
    }
}

/// Emits `PlanItemDone` when every ticket of the item is Closed (Cancelled ones do not count as done work but do
/// not block the item either).
fn plan_item_done(w: &mut WriteCtx<'_, '_>, t: &Ticket) -> Result<()> {
    let f = tickets::TicketFilter {
        plan_item: Some(t.plan_item.clone()),
        ..Default::default()
    };
    let all = tickets::list(w.tx, t.project, &f)?;
    let done = all.iter().all(|x| x.status.is_terminal())
        && all.iter().any(|x| x.status == TicketStatus::Closed);
    if done {
        w.emit_as(
            "system",
            DomainEvent::PlanItemDone {
                project: t.project,
                item: t.plan_item.clone(),
            },
        )?;
    }
    Ok(())
}
