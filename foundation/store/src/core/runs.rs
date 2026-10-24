//! Runs (one execution of an agent loop), the structured run log, and permission grants.

use agenium_core::{AgentId, ProjectId, RunId, TicketId, UsageSummary};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::{from_json, parse, to_json};
use crate::StoreError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    pub ticket: Option<TicketId>,
    pub agent: AgentId,
    pub provider: String,
    pub model: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub outcome: Option<String>,
    pub steps: u32,
    pub usage: UsageSummary,
    pub heartbeat: Option<String>,
}

const COLS: &str =
    "id, ticket, agent, provider, model, started_at, ended_at, outcome, steps, usage, heartbeat";

type Raw = (
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    String,
    Option<String>,
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
        r.get(10)?,
    ))
}

fn decode(v: Raw) -> Result<Run, StoreError> {
    let (
        id,
        ticket,
        agent,
        provider,
        model,
        started_at,
        ended_at,
        outcome,
        steps,
        usage,
        heartbeat,
    ) = v;
    Ok(Run {
        id: parse(&id, "run id")?,
        ticket: ticket
            .as_deref()
            .map(|t| parse(t, "ticket id"))
            .transpose()?,
        agent: parse(&agent, "agent id")?,
        provider,
        model,
        started_at,
        ended_at,
        outcome,
        steps: steps as u32,
        usage: from_json(&usage)?,
        heartbeat,
    })
}

pub fn insert(c: &Connection, r: &Run) -> Result<(), StoreError> {
    c.execute(
        &format!("INSERT INTO run({COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"),
        params![
            r.id.to_string(),
            r.ticket.map(|t| t.to_string()),
            r.agent.to_string(),
            r.provider,
            r.model,
            r.started_at,
            r.ended_at,
            r.outcome,
            r.steps as i64,
            to_json(&r.usage)?,
            r.heartbeat
        ],
    )?;
    Ok(())
}

pub fn get(c: &Connection, id: RunId) -> Result<Run, StoreError> {
    c.query_row(
        &format!("SELECT {COLS} FROM run WHERE id = ?1"),
        [id.to_string()],
        map,
    )
    .optional()?
    .map(decode)
    .transpose()?
    .ok_or_else(|| StoreError::NotFound(format!("run {id}")))
}

/// Records progress of a live run (steps, usage, heartbeat).
pub fn update_progress(
    c: &Connection,
    id: RunId,
    steps: u32,
    usage: &UsageSummary,
    heartbeat: &str,
) -> Result<(), StoreError> {
    c.execute(
        "UPDATE run SET steps = ?2, usage = ?3, heartbeat = ?4 WHERE id = ?1",
        params![id.to_string(), steps as i64, to_json(usage)?, heartbeat],
    )?;
    Ok(())
}

pub fn finish(c: &Connection, id: RunId, ended_at: &str, outcome: &str) -> Result<(), StoreError> {
    c.execute(
        "UPDATE run SET ended_at = ?2, outcome = ?3 WHERE id = ?1 AND ended_at IS NULL",
        params![id.to_string(), ended_at, outcome],
    )?;
    Ok(())
}

pub fn for_ticket(c: &Connection, ticket: TicketId) -> Result<Vec<Run>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} FROM run WHERE ticket = ?1 ORDER BY started_at, id"
    ))?;
    let rows = st.query_map([ticket.to_string()], map)?;
    rows.map(|r| decode(r?)).collect()
}

pub fn for_agent(c: &Connection, agent: AgentId, limit: usize) -> Result<Vec<Run>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} FROM run WHERE agent = ?1 ORDER BY started_at DESC, id DESC LIMIT ?2"
    ))?;
    let rows = st.query_map(params![agent.to_string(), limit as i64], map)?;
    rows.map(|r| decode(r?)).collect()
}

/// The latest runs of every agent, newest first.
pub fn recent(c: &Connection, limit: usize) -> Result<Vec<Run>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} FROM run ORDER BY started_at DESC, id DESC LIMIT ?1"
    ))?;
    let rows = st.query_map(params![limit as i64], map)?;
    rows.map(|r| decode(r?)).collect()
}

/// Runs without `ended_at` (live, or orphaned by a crash).
pub fn unfinished(c: &Connection) -> Result<Vec<Run>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} FROM run WHERE ended_at IS NULL ORDER BY started_at"
    ))?;
    let rows = st.query_map([], map)?;
    rows.map(|r| decode(r?)).collect()
}

// ---------------------------------------------------------------- run log

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogEntry {
    pub n: u32,
    pub kind: String,
    pub data: serde_json::Value,
    pub at: String,
}

pub fn log(
    c: &Connection,
    run: RunId,
    kind: &str,
    data: &serde_json::Value,
) -> Result<u32, StoreError> {
    let n: i64 = c.query_row(
        "SELECT COALESCE(MAX(n), 0) + 1 FROM run_log WHERE run = ?1",
        [run.to_string()],
        |r| r.get(0),
    )?;
    c.execute(
        "INSERT INTO run_log(run, n, kind, data, at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            run.to_string(),
            n,
            kind,
            to_json(data)?,
            agenium_core::now_iso()
        ],
    )?;
    Ok(n as u32)
}

pub fn log_entries(c: &Connection, run: RunId) -> Result<Vec<LogEntry>, StoreError> {
    let mut st =
        c.prepare_cached("SELECT n, kind, data, at FROM run_log WHERE run = ?1 ORDER BY n")?;
    let rows = st.query_map([run.to_string()], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get(1)?,
            r.get::<_, String>(2)?,
            r.get(3)?,
        ))
    })?;
    rows.map(|r| {
        let (n, kind, data, at) = r?;
        Ok(LogEntry {
            n: n as u32,
            kind,
            data: from_json(&data)?,
            at,
        })
    })
    .collect()
}

// ---------------------------------------------------------------- permission grants

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grant {
    pub id: String,
    pub project: ProjectId,
    pub ticket: Option<TicketId>,
    pub run: Option<RunId>,
    /// `run` (Lead or user, this run only) or `project` (always-allow: user only, enforced by a trigger).
    pub scope: String,
    pub granted_by: String,
    pub request: serde_json::Value,
}

pub fn insert_grant(c: &Connection, g: &Grant) -> Result<(), StoreError> {
    c.execute(
        "INSERT INTO permission_grant(id, project, ticket, run, scope, granted_by, request) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            g.id,
            g.project.to_string(),
            g.ticket.map(|t| t.to_string()),
            g.run.map(|r| r.to_string()),
            g.scope,
            g.granted_by,
            to_json(&g.request)?
        ],
    )
    .map_err(|e| match e {
        rusqlite::Error::SqliteFailure(_, Some(msg)) if msg.contains("can only be granted") || msg.contains("needs a run") => {
            StoreError::Invalid(msg)
        }
        e => e.into(),
    })?;
    Ok(())
}

/// Revokes the run-scoped grants of a ticket (it left the working states).
pub fn revoke_ticket_grants(
    c: &Connection,
    ticket: TicketId,
    at: &str,
) -> Result<usize, StoreError> {
    Ok(c.execute(
        "UPDATE permission_grant SET revoked_at = ?2 WHERE ticket = ?1 AND scope = 'run' AND revoked_at IS NULL",
        params![ticket.to_string(), at],
    )?)
}

/// Grants usable while working on `ticket`: its live run-scoped grants plus the project's always-allow grants.
pub fn active_grants(
    c: &Connection,
    project: ProjectId,
    ticket: Option<TicketId>,
) -> Result<Vec<Grant>, StoreError> {
    let mut st = c.prepare_cached(
        "SELECT id, project, ticket, run, scope, granted_by, request FROM permission_grant
         WHERE project = ?1 AND (scope = 'project' OR (ticket = ?2 AND revoked_at IS NULL)) ORDER BY created_at",
    )?;
    let rows = st.query_map(
        params![project.to_string(), ticket.map(|t| t.to_string())],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get::<_, String>(6)?,
            ))
        },
    )?;
    rows.map(|r| {
        let (id, project, ticket, run, scope, granted_by, request) = r?;
        Ok(Grant {
            id,
            project: parse(&project, "project id")?,
            ticket: ticket
                .as_deref()
                .map(|t| parse(t, "ticket id"))
                .transpose()?,
            run: run.as_deref().map(|r| parse(r, "run id")).transpose()?,
            scope,
            granted_by,
            request: from_json(&request)?,
        })
    })
    .collect()
}
