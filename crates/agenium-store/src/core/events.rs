//! Event log: append inside the committing transaction; read for UI resync / replay / diagnostics.

use agenium_core::{now_iso, DomainEvent, EventRecord, TicketId};
use rusqlite::{params, Connection, Row, Transaction};

use super::{from_json, parse};
use crate::StoreError;

pub(crate) fn append(
    tx: &Transaction<'_>,
    intent: Option<&str>,
    actor: &str,
    e: DomainEvent,
) -> Result<EventRecord, StoreError> {
    let ticket = e.ticket();
    let (kind, payload) = e.into_parts();
    let seq: i64 = tx.query_row(
        "UPDATE counter SET n = n + 1 WHERE name = 'event' RETURNING n",
        [],
        |r| r.get(0),
    )?;
    let at = now_iso();
    tx.execute(
        "INSERT INTO event(seq, intent_id, kind, ticket, actor, payload, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![seq, intent, kind, ticket.map(|t| t.to_string()), actor, payload.to_string(), at],
    )?;
    Ok(EventRecord {
        seq: seq as u64,
        intent_id: intent.map(str::to_owned),
        kind,
        ticket,
        actor: actor.to_owned(),
        payload,
        at,
    })
}

type RawEvent = (
    i64,
    Option<String>,
    String,
    Option<String>,
    String,
    String,
    String,
);

fn row(r: &Row<'_>) -> rusqlite::Result<RawEvent> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
    ))
}

fn decode(t: RawEvent) -> Result<EventRecord, StoreError> {
    let (seq, intent_id, kind, ticket, actor, payload, at) = t;
    Ok(EventRecord {
        seq: seq as u64,
        intent_id,
        kind,
        ticket: ticket
            .as_deref()
            .map(|t| parse::<TicketId>(t, "ticket id"))
            .transpose()?,
        actor,
        payload: from_json(&payload)?,
        at,
    })
}

const COLS: &str = "seq, intent_id, kind, ticket, actor, payload, at";

/// Events with `seq > after`, oldest first, at most `limit`.
pub fn since(c: &Connection, after: u64, limit: usize) -> Result<Vec<EventRecord>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} FROM event WHERE seq > ?1 ORDER BY seq LIMIT ?2"
    ))?;
    let rows = st.query_map(params![after as i64, limit as i64], row)?;
    rows.map(|r| decode(r?)).collect()
}

pub fn for_ticket(c: &Connection, ticket: TicketId) -> Result<Vec<EventRecord>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} FROM event WHERE ticket = ?1 ORDER BY seq"
    ))?;
    let rows = st.query_map([ticket.to_string()], row)?;
    rows.map(|r| decode(r?)).collect()
}

pub fn for_intent(c: &Connection, intent: &str) -> Result<Vec<EventRecord>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} FROM event WHERE intent_id = ?1 ORDER BY seq"
    ))?;
    let rows = st.query_map([intent], row)?;
    rows.map(|r| decode(r?)).collect()
}

/// Highest allocated `seq` (0 when empty).
pub fn last_seq(c: &Connection) -> Result<u64, StoreError> {
    let n: i64 = c.query_row("SELECT n FROM counter WHERE name = 'event'", [], |r| {
        r.get(0)
    })?;
    Ok(n as u64)
}

/// Events of a kind, newest first (diagnostics).
pub fn recent_of_kind(
    c: &Connection,
    kind: &str,
    limit: usize,
) -> Result<Vec<EventRecord>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} FROM event WHERE kind = ?1 ORDER BY seq DESC LIMIT ?2"
    ))?;
    let rows = st.query_map(params![kind, limit as i64], row)?;
    rows.map(|r| decode(r?)).collect()
}
