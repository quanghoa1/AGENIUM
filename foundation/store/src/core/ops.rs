//! Journal of git side effects (`vcs_op`).

use agenium_core::TicketId;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{from_json, parse, to_json};
use crate::StoreError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VcsOpRow {
    pub seq: i64,
    pub ticket: TicketId,
    pub op: String,
    pub detail: serde_json::Value,
    pub state: String,
    pub result: Option<serde_json::Value>,
}

pub fn insert(
    c: &Connection,
    seq: i64,
    ticket: TicketId,
    op: &str,
    detail: &serde_json::Value,
) -> Result<(), StoreError> {
    c.execute(
        "INSERT INTO vcs_op(seq, ticket, op, detail, state) VALUES (?1, ?2, ?3, ?4, 'pending')",
        params![seq, ticket.to_string(), op, to_json(detail)?],
    )?;
    Ok(())
}

/// Marks an operation done/failed; returns false when it was not pending (already completed).
pub fn complete(
    c: &Connection,
    seq: i64,
    state: &str,
    result: &serde_json::Value,
) -> Result<bool, StoreError> {
    let n = c.execute(
        "UPDATE vcs_op SET state = ?2, result = ?3, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE seq = ?1 AND state = 'pending'",
        params![seq, state, to_json(result)?],
    )?;
    Ok(n > 0)
}

type Raw = (i64, String, String, String, String, Option<String>);

fn decode(r: Raw) -> Result<VcsOpRow, StoreError> {
    let (seq, ticket, op, detail, state, result) = r;
    Ok(VcsOpRow {
        seq,
        ticket: parse(&ticket, "ticket id")?,
        op,
        detail: from_json(&detail)?,
        state,
        result: result.as_deref().map(from_json).transpose()?,
    })
}

const COLS: &str = "seq, ticket, op, detail, state, result";

pub fn get(c: &Connection, seq: i64) -> Result<Option<VcsOpRow>, StoreError> {
    c.query_row(
        &format!("SELECT {COLS} FROM vcs_op WHERE seq = ?1"),
        [seq],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )
    .optional()?
    .map(decode)
    .transpose()
}

pub fn pending(c: &Connection) -> Result<Vec<VcsOpRow>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} FROM vcs_op WHERE state = 'pending' ORDER BY seq"
    ))?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        ))
    })?;
    rows.map(|r| decode(r?)).collect()
}

pub fn for_ticket(c: &Connection, ticket: TicketId) -> Result<Vec<VcsOpRow>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} FROM vcs_op WHERE ticket = ?1 ORDER BY seq"
    ))?;
    let rows = st.query_map([ticket.to_string()], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        ))
    })?;
    rows.map(|r| decode(r?)).collect()
}
