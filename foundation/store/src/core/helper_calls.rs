//! The model calls of the app's own helpers (ADR-151/152): no agent owns them, the usage views count them.

use agenium_core::UsageSummary;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use super::{from_json, to_json};
use crate::StoreError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelperCall {
    pub id: String,
    /// `design` or `skill`.
    pub kind: String,
    pub provider: String,
    pub model: String,
    /// Through an API provider (a vendor CLI otherwise).
    pub api: bool,
    /// The helper session it belongs to.
    #[serde(default)]
    pub session: Option<String>,
    pub started_at: String,
    pub ended_at: String,
    pub usage: UsageSummary,
}

pub fn insert(c: &Connection, h: &HelperCall) -> Result<(), StoreError> {
    c.execute(
        "INSERT INTO helper_call (id, kind, provider, model, api, session, started_at, ended_at, usage)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            h.id,
            h.kind,
            h.provider,
            h.model,
            h.api as i64,
            h.session,
            h.started_at,
            h.ended_at,
            to_json(&h.usage)?
        ],
    )?;
    Ok(())
}

/// The latest calls, newest first.
pub fn recent(c: &Connection, limit: usize) -> Result<Vec<HelperCall>, StoreError> {
    let mut st = c.prepare_cached(
        "SELECT id, kind, provider, model, api, session, started_at, ended_at, usage FROM helper_call
         ORDER BY started_at DESC, id DESC LIMIT ?1",
    )?;
    let rows = st.query_map(params![limit as i64], |r| {
        Ok(HelperCall {
            id: r.get(0)?,
            kind: r.get(1)?,
            provider: r.get(2)?,
            model: r.get(3)?,
            api: r.get::<_, i64>(4)? != 0,
            session: r.get(5)?,
            started_at: r.get(6)?,
            ended_at: r.get(7)?,
            usage: UsageSummary::default(),
        })
        .map(|h| (h, r.get::<_, String>(8)))
    })?;
    rows.map(|r| {
        let (mut h, usage) = r?;
        h.usage = from_json(&usage?)?;
        Ok(h)
    })
    .collect()
}
