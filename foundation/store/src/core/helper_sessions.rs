//! Helper chats kept to be reopened (ADR-152): the team designer's and the skill writer's conversations.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::StoreError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelperSession {
    pub id: String,
    /// `design` or `skill`.
    pub kind: String,
    pub title: String,
    /// The user named it: its title no longer follows its draft.
    #[serde(default)]
    pub named: bool,
    /// The model it runs on (provider, model, effort, backend).
    pub model: Value,
    /// What the model is sent back: the user's messages and the helper's whole answers.
    pub turns: Value,
    /// What the user saw (replies, questions, which model answered).
    pub transcript: Value,
    /// The attached documents, as Markdown.
    pub attachments: Value,
    /// The last draft.
    pub draft: Option<Value>,
    /// What it started from (`{"proposal": id}`, `{"seed": {...}}`).
    pub source: Option<Value>,
    /// `open`, `saved` (something was saved from it) or `closed`.
    pub status: String,
    /// What was saved from it (a skill or a template id).
    pub result: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

const COLS: &str =
    "id, kind, title, model, turns, transcript, attachments, draft, source, status, result, \
     created_at, updated_at, named";

fn json(s: String) -> Value {
    serde_json::from_str(&s).unwrap_or(Value::Null)
}

fn map(r: &rusqlite::Row) -> rusqlite::Result<HelperSession> {
    Ok(HelperSession {
        id: r.get(0)?,
        kind: r.get(1)?,
        title: r.get(2)?,
        model: json(r.get(3)?),
        turns: json(r.get(4)?),
        transcript: json(r.get(5)?),
        attachments: json(r.get(6)?),
        draft: r.get::<_, Option<String>>(7)?.map(json),
        source: r.get::<_, Option<String>>(8)?.map(json),
        status: r.get(9)?,
        result: r.get(10)?,
        created_at: r.get(11)?,
        updated_at: r.get(12)?,
        named: r.get::<_, i64>(13)? != 0,
    })
}

/// Writes the session whole (a new one, or its next turn).
pub fn put(c: &Connection, s: &HelperSession) -> Result<(), StoreError> {
    c.execute(
        &format!(
            "INSERT INTO helper_session ({COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(id) DO UPDATE SET title = excluded.title, named = excluded.named, model = excluded.model,
               turns = excluded.turns,
               transcript = excluded.transcript, attachments = excluded.attachments, draft = excluded.draft,
               source = excluded.source, status = excluded.status, result = excluded.result,
               updated_at = excluded.updated_at"
        ),
        params![
            s.id,
            s.kind,
            s.title,
            s.model.to_string(),
            s.turns.to_string(),
            s.transcript.to_string(),
            s.attachments.to_string(),
            s.draft.as_ref().map(Value::to_string),
            s.source.as_ref().map(Value::to_string),
            s.status,
            s.result,
            s.created_at,
            s.updated_at,
            s.named as i64
        ],
    )?;
    Ok(())
}

pub fn get(c: &Connection, id: &str) -> Result<Option<HelperSession>, StoreError> {
    Ok(c.query_row(
        &format!("SELECT {COLS} FROM helper_session WHERE id = ?1"),
        params![id],
        map,
    )
    .optional()?)
}

/// The sessions of `kind`, last changed first.
pub fn list(c: &Connection, kind: &str, limit: usize) -> Result<Vec<HelperSession>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} FROM helper_session WHERE kind = ?1 ORDER BY updated_at DESC LIMIT ?2"
    ))?;
    let rows = st.query_map(params![kind, limit as i64], map)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn remove(c: &Connection, id: &str) -> Result<(), StoreError> {
    c.execute("DELETE FROM helper_session WHERE id = ?1", params![id])?;
    Ok(())
}
