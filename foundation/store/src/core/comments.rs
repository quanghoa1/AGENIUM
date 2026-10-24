//! Typed comments on tickets (the audit trail and the only inter-agent channel).

use agenium_core::{Author, Comment, CommentBody, CommentId, TicketId};
use rusqlite::{params, Connection, OptionalExtension, Row};

use super::{from_json, parse, to_json};
use crate::StoreError;

const COLS: &str = "c.id, c.ticket, t.key, c.kind, c.author, c.reply_to, c.body, c.blocking, c.resolved, c.created_at";
const FROM: &str = "FROM comment c JOIN ticket t ON t.id = c.ticket";

type Raw = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    bool,
    bool,
    String,
);

fn map(r: &Row<'_>) -> rusqlite::Result<Raw> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
    ))
}

fn decode(v: Raw) -> Result<Comment, StoreError> {
    let (id, ticket, key, kind, author, reply_to, body, blocking, resolved, created_at) = v;
    Ok(Comment {
        id: parse(&id, "comment id")?,
        ticket: parse(&ticket, "ticket id")?,
        ticket_key: parse(&key, "ticket key")?,
        kind: parse(&kind, "comment kind")?,
        author: parse::<Author>(&author, "author")?,
        reply_to: reply_to
            .as_deref()
            .map(|r| parse(r, "comment id"))
            .transpose()?,
        body: from_json::<CommentBody>(&body)?,
        blocking,
        resolved,
        created_at,
    })
}

/// Inserts a comment. `blocking` is derived from the body (open blocking Question / undecided Permission).
pub fn insert(
    c: &Connection,
    id: CommentId,
    ticket: TicketId,
    author: Author,
    reply_to: Option<CommentId>,
    body: &CommentBody,
    created_at: &str,
) -> Result<Comment, StoreError> {
    c.execute(
        "INSERT INTO comment(id, ticket, kind, author, reply_to, body, blocking, resolved, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)",
        params![
            id.to_string(),
            ticket.to_string(),
            body.kind().as_str(),
            author.to_string(),
            reply_to.map(|r| r.to_string()),
            to_json(body)?,
            body.is_blocking(),
            created_at
        ],
    )?;
    get(c, id)
}

pub fn get(c: &Connection, id: CommentId) -> Result<Comment, StoreError> {
    find(c, id)?.ok_or_else(|| StoreError::NotFound(format!("comment {id}")))
}

pub fn find(c: &Connection, id: CommentId) -> Result<Option<Comment>, StoreError> {
    c.query_row(
        &format!("SELECT {COLS} {FROM} WHERE c.id = ?1"),
        [id.to_string()],
        map,
    )
    .optional()?
    .map(decode)
    .transpose()
}

pub fn for_ticket(c: &Connection, ticket: TicketId) -> Result<Vec<Comment>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} {FROM} WHERE c.ticket = ?1 ORDER BY c.created_at, c.rowid"
    ))?;
    let rows = st.query_map([ticket.to_string()], map)?;
    rows.map(|r| decode(r?)).collect()
}

/// Open (unresolved) blocking comments of a ticket.
pub fn open_blocking(c: &Connection, ticket: TicketId) -> Result<Vec<Comment>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} {FROM} WHERE c.ticket = ?1 AND c.blocking = 1 AND c.resolved = 0 ORDER BY c.created_at, c.rowid"
    ))?;
    let rows = st.query_map([ticket.to_string()], map)?;
    rows.map(|r| decode(r?)).collect()
}

/// Unresolved Question/Permission/Report comments of a project's tickets (the Lead inbox; decision 20).
pub fn inbox(c: &Connection, project: agenium_core::ProjectId) -> Result<Vec<Comment>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} {FROM} WHERE t.project = ?1 AND c.resolved = 0 AND c.kind IN ('Question','Permission','Report')
         ORDER BY c.blocking DESC, c.created_at, c.rowid"
    ))?;
    let rows = st.query_map([project.to_string()], map)?;
    rows.map(|r| decode(r?)).collect()
}

pub fn set_resolved(c: &Connection, id: CommentId, resolved: bool) -> Result<(), StoreError> {
    let n = c.execute(
        "UPDATE comment SET resolved = ?2 WHERE id = ?1",
        params![id.to_string(), resolved],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("comment {id}")));
    }
    Ok(())
}

/// Replaces the body (only used to record a Permission outcome); recomputes `blocking`.
pub fn set_body(c: &Connection, id: CommentId, body: &CommentBody) -> Result<(), StoreError> {
    c.execute(
        "UPDATE comment SET body = ?2, blocking = ?3 WHERE id = ?1",
        params![id.to_string(), to_json(body)?, body.is_blocking()],
    )?;
    Ok(())
}
