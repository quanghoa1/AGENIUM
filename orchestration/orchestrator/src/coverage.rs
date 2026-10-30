//! Plan coverage (ADR-147): a project is done only when the work covers what was agreed.
//! - Every requirement of the agreed spec has a plan item tracing to it (a plan that leaves one out is refused).
//! - A ticket follows its plan item's dependencies: an item that depends on another starts from that item's work.
//! - A cancelled ticket's work is accounted for, or its plan item is not done: it is carried on by a replacement
//!   ticket (`replaced_by` on the cancel, or a new ticket that `follows` it) that closes, or the user waives it — a
//!   cancel by the user is a waiver, a cancel by the Lead without a replacement waits for the user's word.

use std::collections::{HashMap, HashSet};

use agenium_core::*;
use agenium_store::core::{ledger, plans, tickets, WriteCtx};
use agenium_store::StoreError;
use rusqlite::Connection;

use crate::app::App;
use crate::engine::system_comment;
use crate::{OrchError, Result};

fn invalid(m: impl Into<String>) -> OrchError {
    OrchError::Invalid(m.into())
}

/// Every requirement of `spec` has a plan item tracing to it.
pub(crate) fn check_requirements_covered(
    spec: &plans::SpecRow,
    items: &[agenium_core::plan::PlanItem],
) -> Result<()> {
    let missing = uncovered(spec, items);
    if missing.is_empty() {
        return Ok(());
    }
    Err(invalid(format!(
        "no plan item traces to {} of spec v{}: every requirement needs an item (trace_to), or the user changes the spec",
        missing.join(", "),
        spec.version
    )))
}

/// The requirements of `spec` no item of `items` traces to.
pub fn uncovered(spec: &plans::SpecRow, items: &[agenium_core::plan::PlanItem]) -> Vec<String> {
    let traced: HashSet<&str> = items
        .iter()
        .flat_map(|i| i.trace_to.iter().map(String::as_str))
        .collect();
    spec.body
        .requirements
        .iter()
        .filter(|r| !traced.contains(r.id.as_str()))
        .map(|r| r.id.clone())
        .collect()
}

/// A new ticket of `item` must start from the work of the items it depends on: each such item has a ticket already,
/// and its open tickets are among the new ticket's dependencies (direct or through others).
pub(crate) fn check_item_dependencies(
    c: &Connection,
    project: ProjectId,
    plan: &agenium_core::plan::TicketPlan,
    item: &str,
    ancestors: &HashSet<TicketId>,
) -> Result<()> {
    let Some(it) = plan.items.iter().find(|i| i.id == item) else {
        return Ok(());
    };
    if it.depends_on.is_empty() {
        return Ok(());
    }
    let all = tickets::list(c, project, &Default::default())?;
    for dep in &it.depends_on {
        let mine: Vec<&Ticket> = all
            .iter()
            .filter(|t| &t.plan_item == dep && t.status != TicketStatus::Cancelled)
            .collect();
        if mine.is_empty() {
            return Err(invalid(format!(
                "plan item {item} depends on {dep}, which has no ticket yet: create {dep}'s ticket first, then this one with it in depends_on"
            )));
        }
        let open: Vec<String> = mine
            .iter()
            .filter(|t| !t.status.is_terminal() && !ancestors.contains(&t.id))
            .map(|t| t.key.to_string())
            .collect();
        if !open.is_empty() {
            return Err(invalid(format!(
                "plan item {item} depends on {dep}: add {} to depends_on, so this ticket starts from that work",
                open.join(", ")
            )));
        }
    }
    Ok(())
}

/// How a ticket counts for its plan item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accounted {
    /// Closed, or cancelled and carried on by a replacement that closed.
    Done,
    /// Cancelled, and the user agreed the work is not needed.
    Waived,
    /// Open, or cancelled with its work not accounted for yet.
    No,
}

/// How `t` counts for its plan item (following replacements, at most a few deep).
pub fn accounted(c: &Connection, t: &Ticket) -> std::result::Result<Accounted, StoreError> {
    let mut t = t.clone();
    for _ in 0..8 {
        match t.status {
            TicketStatus::Closed => return Ok(Accounted::Done),
            TicketStatus::Cancelled => {}
            _ => return Ok(Accounted::No),
        }
        let Some(x) = ledger::cancellation(c, t.id)? else {
            return Ok(Accounted::No);
        };
        if x.waived_by.is_some() {
            return Ok(Accounted::Waived);
        }
        let Some(next) = x.replaced_by else {
            return Ok(Accounted::No);
        };
        t = tickets::get(c, next)?;
    }
    Ok(Accounted::No)
}

/// `t` is cancelled and nothing carries its work on or waives it: its item waits for a replacement or the user.
pub fn unaccounted(c: &Connection, t: &Ticket) -> std::result::Result<bool, StoreError> {
    if t.status != TicketStatus::Cancelled {
        return Ok(false);
    }
    Ok(ledger::cancellation(c, t.id)?
        .is_none_or(|x| x.replaced_by.is_none() && x.waived_by.is_none()))
}

/// The latest cancelled ticket of `item` whose work nothing carries on: a new ticket of the item that names no other
/// `follows` becomes its replacement (a Lead re-creating cancelled work rarely names it; e2e of 2026-10-30).
pub(crate) fn unaccounted_of_item(
    c: &Connection,
    project: ProjectId,
    item: &str,
) -> std::result::Result<Option<Ticket>, StoreError> {
    let f = tickets::TicketFilter {
        statuses: vec![TicketStatus::Cancelled],
        plan_item: Some(item.to_owned()),
        ..Default::default()
    };
    for t in tickets::list(c, project, &f)?.into_iter().rev() {
        if unaccounted(c, &t)? {
            return Ok(Some(t));
        }
    }
    Ok(None)
}

/// The cancel of `t` by `actor` on record: the user's cancel is a waiver; the Lead's names the ticket that carries the
/// work on, or waits for the user (told in Needs you).
pub(crate) fn record_cancel(
    w: &mut WriteCtx<'_, '_>,
    t: &Ticket,
    actor: &Actor,
    replaced_by: Option<&Ticket>,
) -> Result<()> {
    let user = *actor == Actor::User;
    ledger::record_cancel(
        w.tx,
        &ledger::Cancellation {
            ticket: t.id,
            cancelled_by: if user { "user" } else { "lead" }.into(),
            replaced_by: replaced_by.map(|r| r.id),
            waived_by: user.then(|| "user".into()),
            note: None,
            at: now_iso(),
        },
    )?;
    if !user && replaced_by.is_none() {
        w.emit_as(
            "system",
            DomainEvent::NeedsUser {
                ticket: Some(t.id),
                reason: format!(
                    "The Lead cancelled {} ({}) and no ticket carries its work on: waive it on the ticket page if it is \
                     not needed; otherwise the Lead creates the ticket that does (follows {}).",
                    t.key, t.title, t.key
                ),
            },
        )?;
    }
    Ok(())
}

/// A new ticket that `follows` a cancelled one carries its work on: the cancel gets it as replacement (unless the
/// user waived it already).
pub(crate) fn replaced_by_follower(
    w: &mut WriteCtx<'_, '_>,
    cancelled: &Ticket,
    follower: &Ticket,
) -> Result<()> {
    if cancelled.status != TicketStatus::Cancelled {
        return Ok(());
    }
    let mut x = ledger::cancellation(w.tx, cancelled.id)?.unwrap_or(ledger::Cancellation {
        ticket: cancelled.id,
        cancelled_by: "lead".into(),
        replaced_by: None,
        waived_by: None,
        note: None,
        at: now_iso(),
    });
    if x.waived_by.is_none() && x.replaced_by.is_none() {
        x.replaced_by = Some(follower.id);
        ledger::record_cancel(w.tx, &x)?;
        system_comment(
            w,
            cancelled,
            format!("{} carries this ticket's work on.", follower.key),
        )?;
    }
    Ok(())
}

impl App {
    /// The user agrees a cancelled ticket's work is not needed: its plan item no longer waits for it.
    pub fn waive_cancelled(
        &self,
        project: ProjectId,
        key: &TicketKey,
        note: Option<&str>,
    ) -> Result<()> {
        let t = self.ticket(project, key)?;
        if t.status != TicketStatus::Cancelled {
            return Err(invalid(format!(
                "{key} is {}: only a cancelled ticket is waived",
                t.status
            )));
        }
        self.store.commit(None, "user", |w| -> Result<()> {
            if ledger::cancellation(w.tx, t.id)?.is_none() {
                record_cancel(w, &t, &Actor::User, None)?;
            } else {
                ledger::waive(w.tx, t.id, note)?;
            }
            system_comment(
                w,
                &t,
                format!(
                    "The user waived this ticket's work: it is not needed.{}",
                    note.map(|n| format!(" {n}")).unwrap_or_default()
                ),
            )?;
            w.emit(DomainEvent::TicketUpdated {
                ticket: t.id,
                key: t.key.clone(),
                fields: vec!["waived".into()],
            })?;
            Ok(())
        })?;
        Ok(())
    }

    /// Cancelled tickets of `project` whose work nothing accounts for yet (no replacement, no waiver), by plan item.
    pub fn unaccounted_cancels(
        &self,
        project: ProjectId,
    ) -> Result<HashMap<String, Vec<TicketKey>>> {
        let ts = self.tickets(project)?;
        let mut out: HashMap<String, Vec<TicketKey>> = HashMap::new();
        self.store.read(|c| {
            for t in ts.iter().filter(|t| t.status == TicketStatus::Cancelled) {
                if unaccounted(c, t)? {
                    out.entry(t.plan_item.clone())
                        .or_default()
                        .push(t.key.clone());
                }
            }
            Ok(())
        })?;
        Ok(out)
    }
}
