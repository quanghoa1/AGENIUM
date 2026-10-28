//! Permissions and the write scope: requests, the Lead's or the user's decision, and what a ticket may change. Split
//! out of `engine.rs`.

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

    /// Write scope of the assignee on `t` (architecture page "Ép phạm vi"): the ticket's `scope_paths` and the
    /// role's allowed paths must both match; approved path permissions of the ticket and always-allowed project
    /// paths widen it. Returns (lists that must all match, granted patterns).
    pub fn write_scope(&self, t: &Ticket, role: &str) -> Result<(Vec<Vec<String>>, Vec<String>)> {
        let p = self.project(t.project)?;
        let mut granted: Vec<String> = allowlist(self, &p)
            .into_iter()
            .filter_map(|e| e.strip_prefix("path:").map(str::to_owned))
            .collect();
        for g in self
            .store
            .read(|c| runs::active_grants(c, t.project, Some(t.id)))?
        {
            if let Ok(r) = serde_json::from_value::<PermissionRequest>(g.request) {
                if r.kind == PermissionKind::Path {
                    granted.extend(r.paths);
                }
            }
        }
        Ok((
            vec![t.scope_paths.clone(), self.role_allowed_paths(role)],
            granted,
        ))
    }

    /// Files changed on the ticket's branch (committed since its base, and uncommitted) that fall outside the write
    /// scope of the assignee. Covers shell commands and CLI backends, whose writes the file tools never see.
    pub(super) fn out_of_scope_changes(
        &self,
        project: &projects::Project,
        t: &Ticket,
        role: &str,
    ) -> Result<Vec<String>> {
        let (all_of, granted) = self.write_scope(t, role)?;
        let scope =
            agenium_tools::paths::WriteScope::new(all_of, granted).map_err(OrchError::Invalid)?;
        if scope.is_open() {
            return Ok(vec![]);
        }
        let Some(ws) = self
            .store
            .read(|c| workspaces::get(c, t.id))?
            .filter(|w| !w.removed)
        else {
            return Ok(vec![]);
        };
        let repo = self.repo(project);
        let wt = std::path::Path::new(&ws.worktree);
        let base = crate::effects::work_base(&repo, wt, &project.main_branch, &ws.base_sha);
        let mut files = repo.changed_files(wt, &base, "HEAD")?;
        for line in repo.status(wt)? {
            // Porcelain v1: "XY path" or "XY from -> to". Both sides of a rename count: moving a file out of the
            // scope deletes it there.
            let path = line.get(3..).unwrap_or_default();
            files.extend(path.split(" -> ").map(|p| p.trim_matches('"').to_owned()));
        }
        files.sort();
        files.dedup();
        Ok(files
            .into_iter()
            .filter(|f| !f.starts_with("artifacts/tool-output/") && !scope.allows(f))
            .collect())
    }
}
