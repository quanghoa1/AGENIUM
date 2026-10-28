//! Permission requests and the Lead's or the user's decision on them. Split out of `engine.rs`.

use super::*;

impl App {
    pub(super) fn request_permission(
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
            let solo = solo_agent(w.tx, agent)?;
            let decider = if inside && !solo { "lead" } else { "user" };
            if decider == "user" {
                w.emit_as("system", DomainEvent::NeedsUser { ticket: Some(t.id), reason: format!("{} requests a permission outside the allowlist", t.key) })?;
            }
            Ok(Outcome { result: serde_json::json!({ "request_id": c.id, "in_allowlist": inside, "decided_by": decider }), run: RunEffect::Park, ops: vec![] })
        })
    }

    pub(super) fn approve_permission(
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
            let team = require_lead(w, pid, &ctx.actor)?;
            let mut t = ticket_by_key(w, pid, &a.ticket_id)?;
            let c = comments::find(w.tx, a.request_id)?.ok_or_else(|| invalid("request_id does not exist"))?;
            let CommentBody::Permission { request, run, outcome: None } = c.body.clone() else {
                return Err(invalid("request_id is not an undecided permission request"));
            };
            if c.ticket != t.id {
                return Err(invalid("the request belongs to another ticket"));
            }
            if solo_self(team.as_ref(), &ctx.actor, &c) {
                return Err(denied("a Solo agent's own permission requests go to the user: wait for the decision"));
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
}
