//! The project map (ADR-166): nodes and links agents added with their evidence, kept per project.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::StoreError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapNode {
    pub key: String,
    pub kind: String,
    pub name: String,
    pub file: Option<String>,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
    pub summary: String,
    pub evidence: serde_json::Value,
    pub status: String,
    pub ticket: Option<String>,
    pub agent: String,
    pub run: String,
    pub created_at: String,
    pub checked_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapLink {
    pub id: i64,
    pub src: String,
    pub kind: String,
    pub dst: String,
    pub evidence: serde_json::Value,
    pub status: String,
    pub ticket: Option<String>,
    pub agent: String,
    pub run: String,
    pub created_at: String,
    pub checked_at: String,
}

const NODE: &str =
    "key, kind, name, file, line_start, line_end, summary, evidence, status, ticket, agent, run, \
                    created_at, checked_at";
const LINK: &str =
    "id, src, kind, dst, evidence, status, ticket, agent, run, created_at, checked_at";

fn node_row(r: &Row) -> rusqlite::Result<MapNode> {
    Ok(MapNode {
        key: r.get(0)?,
        kind: r.get(1)?,
        name: r.get(2)?,
        file: r.get(3)?,
        line_start: r.get(4)?,
        line_end: r.get(5)?,
        summary: r.get(6)?,
        evidence: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
        status: r.get(8)?,
        ticket: r.get(9)?,
        agent: r.get(10)?,
        run: r.get(11)?,
        created_at: r.get(12)?,
        checked_at: r.get(13)?,
    })
}

fn link_row(r: &Row) -> rusqlite::Result<MapLink> {
    Ok(MapLink {
        id: r.get(0)?,
        src: r.get(1)?,
        kind: r.get(2)?,
        dst: r.get(3)?,
        evidence: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
        status: r.get(5)?,
        ticket: r.get(6)?,
        agent: r.get(7)?,
        run: r.get(8)?,
        created_at: r.get(9)?,
        checked_at: r.get(10)?,
    })
}

/// Writes `n` (added, or replacing the node of the same key).
pub fn put_node(c: &Connection, project: &str, n: &MapNode) -> Result<(), StoreError> {
    c.execute(
        &format!("INSERT OR REPLACE INTO map_node (project, {NODE}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)"),
        params![
            project,
            n.key,
            n.kind,
            n.name,
            n.file,
            n.line_start,
            n.line_end,
            n.summary,
            n.evidence.to_string(),
            n.status,
            n.ticket,
            n.agent,
            n.run,
            n.created_at,
            n.checked_at
        ],
    )?;
    Ok(())
}

pub fn node(c: &Connection, project: &str, key: &str) -> Result<Option<MapNode>, StoreError> {
    Ok(c.query_row(
        &format!("SELECT {NODE} FROM map_node WHERE project = ?1 AND key = ?2"),
        params![project, key],
        node_row,
    )
    .optional()?)
}

pub fn nodes(c: &Connection, project: &str) -> Result<Vec<MapNode>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {NODE} FROM map_node WHERE project = ?1 ORDER BY key"
    ))?;
    let rows = st.query_map([project], node_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Writes `l` (added, or replacing the link between the same nodes of the same kind); its id.
pub fn put_link(c: &Connection, project: &str, l: &MapLink) -> Result<i64, StoreError> {
    c.execute(
        "INSERT INTO map_link (project, src, kind, dst, evidence, status, ticket, agent, run, created_at, checked_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT (project, src, kind, dst) DO UPDATE SET evidence = excluded.evidence, status = excluded.status,
           ticket = excluded.ticket, agent = excluded.agent, run = excluded.run, checked_at = excluded.checked_at",
        params![
            project,
            l.src,
            l.kind,
            l.dst,
            l.evidence.to_string(),
            l.status,
            l.ticket,
            l.agent,
            l.run,
            l.created_at,
            l.checked_at
        ],
    )?;
    Ok(c.query_row(
        "SELECT id FROM map_link WHERE project = ?1 AND src = ?2 AND kind = ?3 AND dst = ?4",
        params![project, l.src, l.kind, l.dst],
        |r| r.get(0),
    )?)
}

pub fn links(c: &Connection, project: &str) -> Result<Vec<MapLink>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {LINK} FROM map_link WHERE project = ?1 ORDER BY id"
    ))?;
    let rows = st.query_map([project], link_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The links from or to `key`.
pub fn links_of(c: &Connection, project: &str, key: &str) -> Result<Vec<MapLink>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {LINK} FROM map_link WHERE project = ?1 AND (src = ?2 OR dst = ?2) ORDER BY kind, id"
    ))?;
    let rows = st.query_map(params![project, key], link_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn remove_link(c: &Connection, project: &str, id: i64) -> Result<(), StoreError> {
    c.execute(
        "DELETE FROM map_link WHERE project = ?1 AND id = ?2",
        params![project, id],
    )?;
    Ok(())
}

/// Removes node `key` and its links.
pub fn remove_node(c: &Connection, project: &str, key: &str) -> Result<(), StoreError> {
    c.execute(
        "DELETE FROM map_link WHERE project = ?1 AND (src = ?2 OR dst = ?2)",
        params![project, key],
    )?;
    c.execute(
        "DELETE FROM map_node WHERE project = ?1 AND key = ?2",
        params![project, key],
    )?;
    Ok(())
}

pub fn set_link_status(
    c: &Connection,
    project: &str,
    id: i64,
    status: &str,
    evidence: &serde_json::Value,
    at: &str,
) -> Result<(), StoreError> {
    c.execute(
        "UPDATE map_link SET status = ?3, evidence = ?4, checked_at = ?5 WHERE project = ?1 AND id = ?2",
        params![project, id, status, evidence.to_string(), at],
    )?;
    Ok(())
}

/// The tickets with tentative entries.
pub fn tentative_tickets(c: &Connection, project: &str) -> Result<Vec<String>, StoreError> {
    let mut st = c.prepare_cached(
        "SELECT ticket FROM map_node WHERE project = ?1 AND status = 'tentative' AND ticket IS NOT NULL
         UNION SELECT ticket FROM map_link WHERE project = ?1 AND status = 'tentative' AND ticket IS NOT NULL",
    )?;
    let rows = st.query_map([project], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Drops what ticket `ticket` added and was never confirmed (the ticket was cancelled): its tentative links, and its
/// tentative nodes that no link touches any more (another ticket's link keeps its end).
pub fn drop_tentative(c: &Connection, project: &str, ticket: &str) -> Result<(), StoreError> {
    c.execute(
        "DELETE FROM map_link WHERE project = ?1 AND ticket = ?2 AND status = 'tentative'",
        params![project, ticket],
    )?;
    c.execute(
        "DELETE FROM map_node WHERE project = ?1 AND ticket = ?2 AND status = 'tentative' AND NOT EXISTS
           (SELECT 1 FROM map_link l WHERE l.project = ?1 AND (l.src = map_node.key OR l.dst = map_node.key))",
        params![project, ticket],
    )?;
    Ok(())
}
