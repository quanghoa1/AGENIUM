//! add_dependency: the Lead makes an open ticket wait for another one found or created while it ran (a fix its QA found:
//! without it the worker could only be told in a comment, and asked again and again whether the fix had merged).

use super::*;
use agenium_core::protocol::AddDependency;

impl App {
    pub(super) fn add_dependency(&self, ctx: &CallCtx, a: AddDependency) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            require_lead(w, pid, &ctx.actor)?;
            let t = ticket_by_key(w, pid, &a.ticket_id)?;
            let d = ticket_by_key(w, pid, &a.depends_on)?;
            if t.status.is_terminal() {
                return Err(invalid(format!(
                    "{} is {}: it waits for nothing any more",
                    t.key, t.status
                )));
            }
            if d.status == TicketStatus::Cancelled {
                return Err(invalid(format!(
                    "{} is cancelled and will never be done",
                    d.key
                )));
            }
            if t.id == d.id {
                return Err(invalid(format!("{} cannot wait for itself", t.key)));
            }
            // `d` must not wait (directly or not) for `t`, or neither would ever start.
            let graph = tickets::dep_graph(w.tx, pid)?;
            if crate::engine_support::closure(&graph, &[d.id]).contains(&t.id) {
                return Err(invalid(format!(
                    "{} already waits for {}: that would be a cycle",
                    d.key, t.key
                )));
            }
            let mut deps: Vec<TicketId> = graph.get(&t.id).cloned().unwrap_or_default();
            if !deps.contains(&d.id) {
                deps.push(d.id);
                tickets::set_deps(w.tx, t.id, &deps)?;
                system_comment(
                    w,
                    &t,
                    format!(
                        "{} now waits for {}: it goes on once {} is done.",
                        t.key, d.key, d.key
                    ),
                )?;
            }
            Ok(Outcome::ok(
                serde_json::json!({ "ticket_id": t.key, "waits_for": d.key }),
            ))
        })
    }
}
