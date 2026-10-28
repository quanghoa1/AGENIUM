//! Work across tickets: handoffs, suggested and confirmed links, aggregates of findings. Split out of `engine.rs`.

use super::*;

impl App {
    pub(super) fn create_handoff(&self, ctx: &CallCtx, h: Handoff) -> Result<Outcome> {
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

    pub(super) fn suggest_link(&self, ctx: &CallCtx, l: SuggestLink) -> Result<Outcome> {
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

    pub(super) fn confirm_link(&self, ctx: &CallCtx, l: ConfirmLink) -> Result<Outcome> {
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

    pub(super) fn create_aggregate(
        &self,
        ctx: &CallCtx,
        project: &projects::Project,
        a: CreateAggregate,
    ) -> Result<Outcome> {
        // An aggregate is an ordinary ticket on the plan item of its first child, linked `aggregates` to each child.
        // The form is validated on parse; checked here too, as `apply` may be called with an unchecked intent.
        if a.children.len() < 2 {
            return Err(invalid(
                "an aggregate links two or more existing tickets (children)",
            ));
        }
        let pid = project.id;
        let default_template = self.default_template(project);
        // The ticket and its links commit together: a child key that does not exist leaves nothing behind.
        self.commit(ctx, |w| {
            require_lead(w, pid, &ctx.actor)?;
            let children = a
                .children
                .iter()
                .map(|k| ticket_by_key(w, pid, k))
                .collect::<Result<Vec<_>>>()?;
            let first = &children[0];
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
                follows: None,
            };
            let agg = insert_ticket(w, pid, nt, default_template)?;
            for child in &children {
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
            Ok(Outcome::ok(serde_json::json!({ "ticket_id": agg.key })))
        })
    }
}
