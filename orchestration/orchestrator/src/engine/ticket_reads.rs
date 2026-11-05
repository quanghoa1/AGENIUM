//! The Lead's (and Solo's) view of the project's tickets mid-run: `list_tickets` (the board and its progress) and
//! `read_ticket` (one ticket whole, with its comments). Read only: nothing is written, no event is emitted. Without
//! them a Lead that lost track of what it created went looking in the shell (2026-11-04, a Grok Lead read the store).

use super::*;
use agenium_core::protocol::{ListTickets, ReadTicket};

/// Comments of a ticket shown at most by `read_ticket` (the newest).
const COMMENTS_SHOWN: usize = 30;

impl App {
    fn require_project_lead(&self, ctx: &CallCtx) -> Result<HashMap<AgentId, String>> {
        let (team, members) = self
            .team_of_project(ctx.project)?
            .ok_or_else(|| invalid("the project has no team"))?;
        if !matches!(ctx.actor, Actor::Lead(a) if team.lead == Some(a)) && ctx.actor != Actor::User
        {
            return Err(denied("only the Lead reads the project's tickets"));
        }
        Ok(members.into_iter().map(|m| (m.id, m.name)).collect())
    }

    pub(super) fn list_tickets(&self, ctx: &CallCtx, q: ListTickets) -> Result<Outcome> {
        let names = self.require_project_lead(ctx)?;
        let wanted: Vec<String> = q.status.iter().map(|s| s.to_lowercase()).collect();
        let all = self.tickets(ctx.project)?;
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        for t in &all {
            *counts.entry(t.status.to_string()).or_default() += 1;
        }
        let shown: Vec<serde_json::Value> = all
            .iter()
            .filter(|t| q.include_finished || !matches!(t.status, TicketStatus::Closed | TicketStatus::Cancelled))
            .filter(|t| wanted.is_empty() || wanted.contains(&t.status.to_string().to_lowercase()))
            .map(|t| {
                serde_json::json!({
                    "ticket_id": t.key, "title": t.title, "status": t.status, "plan_item": t.plan_item,
                    "assignee": t.assignee.and_then(|a| names.get(&a).cloned()), "depends_on": t.depends_on,
                    "scope_paths": t.scope_paths, "approved": t.approved, "merged": t.merged,
                    "reopened": t.reopen_count, "updated_at": t.updated_at,
                })
            })
            .collect();
        Ok(Outcome::ok(serde_json::json!({
            "total": all.len(), "by_status": counts, "tickets": shown,
            "next": "read_ticket gives one ticket whole (description, acceptance, comments, report)",
        })))
    }

    pub(super) fn read_ticket(&self, ctx: &CallCtx, q: ReadTicket) -> Result<Outcome> {
        let names = self.require_project_lead(ctx)?;
        let t = self.ticket(ctx.project, &q.ticket_id)?;
        let cs = self.store.read(|c| comments::for_ticket(c, t.id))?;
        let from = cs.len().saturating_sub(COMMENTS_SHOWN);
        Ok(Outcome::ok(serde_json::json!({
            "ticket_id": t.key, "status": t.status,
            "assignee": t.assignee.and_then(|a| names.get(&a).cloned()),
            "text": agenium_backend::prompt::ticket_text(&t, None),
            "comments": agenium_backend::prompt::comments_text(&cs[from..]),
            "comments_left_out": from,
        })))
    }

    /// read_spec: the approved spec the current plan traces to (else the newest approved one), whole. Workers see only
    /// the requirements their item serves; a ticket that documents or checks the whole contract needs all of it (a docs
    /// worker was told to compare with "the approved v1 contract" it could not read, and reopened for it).
    pub(super) fn read_spec(&self, ctx: &CallCtx) -> Result<Outcome> {
        let spec = self.store.read(|c| {
            let traced = match plans::current_plan(c, ctx.project)? {
                Some(p) => plans::spec(c, ctx.project, p.body.spec_version)?,
                None => None,
            };
            Ok(match traced {
                Some(s) => Some(s),
                None => plans::latest_spec(c, ctx.project)?.filter(|s| s.approved_by.is_some()),
            })
        })?;
        let Some(s) = spec else {
            return Err(invalid("the project has no approved spec yet"));
        };
        Ok(Outcome::ok(serde_json::json!({
            "version": s.version, "approved": s.approved_by.is_some(), "spec": s.body,
        })))
    }

    /// share_image: a screenshot of a ticket's report, shown to the user in the chat (the chat reads it from the call).
    pub(super) fn share_image(
        &self,
        ctx: &CallCtx,
        a: agenium_core::protocol::ShareImage,
    ) -> Result<Outcome> {
        self.require_project_lead(ctx)?;
        let p = self.project(ctx.project)?;
        let t = self.ticket(ctx.project, &a.ticket_id)?;
        if crate::evidence::read(std::path::Path::new(&p.path), t.key.as_str(), &a.image).is_none()
        {
            return Err(invalid(format!(
                "{} has no screenshot {:?}: read_ticket shows its report's screenshots (artifacts of kind screenshot)",
                t.key, a.image
            )));
        }
        Ok(Outcome::ok(
            serde_json::json!({ "shown": true, "ticket_id": t.key, "image": a.image }),
        ))
    }
}
