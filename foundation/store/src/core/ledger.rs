//! What must outlive a crash between a decision and its effect (ADR-147): messages to a Lead until a run of the Lead
//! had them, tickets waiting for their assignee to learn from them, and how a cancelled ticket's work is accounted for.

use agenium_core::{ProjectId, TicketId};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::parse;
use crate::StoreError;

// ---------------------------------------------------------------- messages to a Lead

/// A message to a project's Lead not yet delivered to a run of the Lead.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeadMessage {
    pub id: i64,
    pub text: String,
    pub created_at: String,
}

/// Stores a message to the Lead of `project`; returns its id.
pub fn post_lead_message(
    c: &Connection,
    project: ProjectId,
    text: &str,
    at: &str,
) -> Result<i64, StoreError> {
    c.execute(
        "INSERT INTO lead_message(project, text, created_at) VALUES (?1, ?2, ?3)",
        params![project.to_string(), text, at],
    )?;
    Ok(c.last_insert_rowid())
}

/// The project's undelivered messages to its Lead, oldest first.
pub fn undelivered(c: &Connection, project: ProjectId) -> Result<Vec<LeadMessage>, StoreError> {
    let mut st = c.prepare_cached(
        "SELECT id, text, created_at FROM lead_message WHERE project = ?1 AND delivered_at IS NULL ORDER BY id",
    )?;
    let rows = st
        .query_map(params![project.to_string()], |r| {
            Ok(LeadMessage {
                id: r.get(0)?,
                text: r.get(1)?,
                created_at: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn mark_delivered(c: &Connection, ids: &[i64], at: &str) -> Result<(), StoreError> {
    for id in ids {
        c.execute(
            "UPDATE lead_message SET delivered_at = ?2 WHERE id = ?1 AND delivered_at IS NULL",
            params![id, at],
        )?;
    }
    Ok(())
}

/// Puts `ids` back in the queue (the turn that took them did not reach its end).
pub fn mark_undelivered(c: &Connection, ids: &[i64]) -> Result<(), StoreError> {
    for id in ids {
        c.execute(
            "UPDATE lead_message SET delivered_at = NULL WHERE id = ?1",
            params![id],
        )?;
    }
    Ok(())
}

/// Rewrites a queued message (ADR-149); `false` when the Lead has it already (or it is not this project's).
pub fn edit_queued(
    c: &Connection,
    project: ProjectId,
    id: i64,
    text: &str,
) -> Result<bool, StoreError> {
    Ok(c.execute(
        "UPDATE lead_message SET text = ?3 WHERE id = ?1 AND project = ?2 AND delivered_at IS NULL",
        params![id, project.to_string(), text],
    )? == 1)
}

/// Takes a queued message back (ADR-149); `false` when the Lead has it already (or it is not this project's).
pub fn drop_queued(c: &Connection, project: ProjectId, id: i64) -> Result<bool, StoreError> {
    Ok(c.execute(
        "DELETE FROM lead_message WHERE id = ?1 AND project = ?2 AND delivered_at IS NULL",
        params![id, project.to_string()],
    )? == 1)
}

// ---------------------------------------------------------------- learning owed

/// Records that `ticket` reached `state` (`<status>:<reopen_count>`) and its assignee should learn from it.
pub fn owe_learning(
    c: &Connection,
    ticket: TicketId,
    state: &str,
    at: &str,
) -> Result<(), StoreError> {
    c.execute(
        "INSERT OR IGNORE INTO learn_pending(ticket, state, created_at) VALUES (?1, ?2, ?3)",
        params![ticket.to_string(), state, at],
    )?;
    Ok(())
}

/// Tickets of `project` owed a learning pass, oldest first: (ticket, state).
pub fn learning_owed(
    c: &Connection,
    project: ProjectId,
) -> Result<Vec<(TicketId, String)>, StoreError> {
    let mut st = c.prepare_cached(
        "SELECT l.ticket, l.state FROM learn_pending l JOIN ticket t ON t.id = l.ticket
         WHERE t.project = ?1 ORDER BY l.created_at, l.ticket",
    )?;
    let rows = st
        .query_map(params![project.to_string()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(t, s)| Ok((parse(&t, "ticket id")?, s)))
        .collect()
}

pub fn learning_done(c: &Connection, ticket: TicketId, state: &str) -> Result<(), StoreError> {
    c.execute(
        "DELETE FROM learn_pending WHERE ticket = ?1 AND state = ?2",
        params![ticket.to_string(), state],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- cancelled tickets

/// How a cancelled ticket's work is accounted for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cancellation {
    pub ticket: TicketId,
    /// `user`, `lead` or `legacy`.
    pub cancelled_by: String,
    pub replaced_by: Option<TicketId>,
    /// `user` or `legacy`: the work is not needed.
    pub waived_by: Option<String>,
    pub note: Option<String>,
    pub at: String,
}

pub fn record_cancel(c: &Connection, x: &Cancellation) -> Result<(), StoreError> {
    c.execute(
        "INSERT OR REPLACE INTO ticket_cancel(ticket, cancelled_by, replaced_by, waived_by, note, at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            x.ticket.to_string(),
            x.cancelled_by,
            x.replaced_by.map(|t| t.to_string()),
            x.waived_by,
            x.note,
            x.at
        ],
    )?;
    Ok(())
}

pub fn cancellation(c: &Connection, ticket: TicketId) -> Result<Option<Cancellation>, StoreError> {
    let row = c
        .query_row(
            "SELECT cancelled_by, replaced_by, waived_by, note, at FROM ticket_cancel WHERE ticket = ?1",
            params![ticket.to_string()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    row.map(|(by, replaced, waived, note, at)| {
        Ok(Cancellation {
            ticket,
            cancelled_by: by,
            replaced_by: replaced.map(|t| parse(&t, "ticket id")).transpose()?,
            waived_by: waived,
            note,
            at,
        })
    })
    .transpose()
}

/// The user agrees a cancelled ticket's work is not needed.
pub fn waive(c: &Connection, ticket: TicketId, note: Option<&str>) -> Result<(), StoreError> {
    let n = c.execute(
        "UPDATE ticket_cancel SET waived_by = 'user', note = COALESCE(?2, note) WHERE ticket = ?1",
        params![ticket.to_string(), note],
    )?;
    if n == 0 {
        return Err(StoreError::Conflict(format!(
            "ticket {ticket} has no cancellation on record"
        )));
    }
    Ok(())
}
