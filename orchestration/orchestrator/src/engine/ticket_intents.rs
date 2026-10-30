//! Ticket intents: creating and assigning tickets, progress, the user's priority and comments. Split out of `engine.rs`.

use super::*;

impl App {
    pub(super) fn create_ticket(
        &self,
        ctx: &CallCtx,
        project: &projects::Project,
        nt: NewTicket,
    ) -> Result<Outcome> {
        let pid = project.id;
        let default_template = self.default_template(project);
        let follows = nt.follows.clone();
        self.commit(ctx, |w| {
            require_lead(w, pid, &ctx.actor)?;
            // A follow-up names the closed ticket it goes on from before anything is written.
            let prev = follows
                .as_ref()
                .map(|k| ticket_by_key(w, pid, k))
                .transpose()?;
            let t = insert_ticket(w, pid, nt, default_template)?;
            if let Some(prev) = &prev {
                crate::coverage::replaced_by_follower(w, prev, &t)?;
            }
            if let Some(prev) = prev {
                tickets::add_link(
                    w.tx,
                    t.id,
                    prev.id,
                    LinkKind::FollowUpOf.as_str(),
                    &ctx.actor.label(),
                    Some(&ctx.actor.label()),
                    None,
                )?;
                w.emit(DomainEvent::LinkAdded {
                    ticket: t.id,
                    from: t.key.clone(),
                    to: prev.key.clone(),
                    kind: LinkKind::FollowUpOf.as_str().into(),
                    confirmed: true,
                })?;
            }
            Ok(Outcome::ok(
                serde_json::json!({ "ticket_id": t.key, "status": t.status }),
            ))
        })
    }

    pub(super) fn default_template(&self, project: &projects::Project) -> String {
        self.manifest(project)
            .map(|m| m.project.template)
            .unwrap_or_else(|_| "coding@1".into())
    }

    pub(super) fn assign_ticket(&self, ctx: &CallCtx, a: AssignTicket) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            let team = require_lead(w, pid, &ctx.actor)?.ok_or_else(|| invalid("the project has no team"))?;
            let mut t = ticket_by_key(w, pid, &a.ticket_id)?;
            let agent = agents::find_agent(w.tx, a.worker)?.ok_or_else(|| invalid(format!("unknown agent {}", a.worker)))?;
            if self.is_planner_role(&agent.role) {
                return Err(invalid(format!("{} is the team's planner: it drafts plans and takes no tickets", agent.name)));
            }
            let in_team = agent.team == Some(team.id) && (agent.role_kind == RoleKind::Worker || team.solo);
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
                TicketStatus::Assigned if ctx.actor == Actor::User => {
                    // User override: another assignee before the run starts; the worktree stays with the ticket.
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

    pub(super) fn post_progress(&self, ctx: &CallCtx, p: ProgressUpdate) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            let t = ticket_by_key(w, pid, &p.ticket_id)?;
            require_assignee(&t, &ctx.actor)?;
            let c = add_comment(w, &t, author_of(&ctx.actor), None, CommentBody::Progress(p))?;
            Ok(Outcome::ok(serde_json::json!({ "comment_id": c.id })))
        })
    }

    /// User override of an open ticket's priority (0 = P0 ..= 3); recorded as an event and a System comment.
    pub fn user_set_priority(
        &self,
        project: ProjectId,
        key: &TicketKey,
        priority: u8,
    ) -> Result<Ticket> {
        if priority > 3 {
            return Err(invalid("priority is 0..=3 (P0 highest)"));
        }
        self.store
            .commit(None, "user", |w| -> Result<Ticket> {
                let mut t = ticket_by_key(w, project, key)?;
                if t.status.is_terminal() {
                    return Err(invalid(format!("{} is {}", t.key, t.status)));
                }
                if t.priority != priority {
                    let from = t.priority;
                    t.priority = priority;
                    tickets::update(w.tx, &t)?;
                    w.emit(DomainEvent::TicketUpdated {
                        ticket: t.id,
                        key: t.key.clone(),
                        fields: vec!["priority".into()],
                    })?;
                    system_comment(
                        w,
                        &t,
                        format!("The user changed the priority from P{from} to P{priority}."),
                    )?;
                }
                Ok(t)
            })
            .map(|c| c.value)
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
}
