//! Project registry: a project is a folder (git repository with `agenium.toml`) referenced by path (decision 7).

use std::fmt;
use std::str::FromStr;

use agenium_core::{ProjectId, TicketKey};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::{from_json, parse, to_json};
use crate::StoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectStatus {
    Active,
    /// Agents stopped; the user may edit `main` directly (decision 19).
    Paused,
    Completed,
    Cancelled,
}

impl ProjectStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Paused => "Paused",
            Self::Completed => "Completed",
            Self::Cancelled => "Cancelled",
        }
    }
}

impl fmt::Display for ProjectStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ProjectStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        [Self::Active, Self::Paused, Self::Completed, Self::Cancelled]
            .into_iter()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| format!("unknown project status {s:?}"))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub path: String,
    pub key_prefix: String,
    pub next_seq: u64,
    pub status: ProjectStatus,
    pub main_branch: String,
    pub git: serde_json::Value,
    /// Always-allow entries (domains/paths/commands) granted by the user.
    pub allowlist: Vec<String>,
    pub settings: serde_json::Value,
    pub created_at: String,
}

impl Project {
    pub fn review_before_merge(&self) -> bool {
        self.settings
            .get("review_before_merge")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }
}

const COLS: &str = "id, name, path, key_prefix, next_seq, status, main_branch, git, allowlist, settings, created_at";

fn map(r: &Row<'_>) -> rusqlite::Result<[String; 11]> {
    Ok([
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get::<_, i64>(4)?.to_string(),
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
    ])
}

fn decode(v: [String; 11]) -> Result<Project, StoreError> {
    let [id, name, path, key_prefix, next_seq, status, main_branch, git, allowlist, settings, created_at] =
        v;
    Ok(Project {
        id: parse(&id, "project id")?,
        name,
        path,
        key_prefix,
        next_seq: parse(&next_seq, "next_seq")?,
        status: parse(&status, "project status")?,
        main_branch,
        git: from_json(&git)?,
        allowlist: from_json(&allowlist)?,
        settings: from_json(&settings)?,
        created_at,
    })
}

#[derive(Debug, Clone)]
pub struct NewProject {
    pub id: ProjectId,
    pub name: String,
    pub path: String,
    pub key_prefix: String,
    pub main_branch: String,
}

pub fn insert(c: &Connection, p: &NewProject) -> Result<Project, StoreError> {
    if !TicketKey::valid_prefix(&p.key_prefix) {
        return Err(StoreError::Invalid(format!(
            "key prefix {:?} must be 1-10 uppercase letters",
            p.key_prefix
        )));
    }
    let n = c.execute(
        "INSERT INTO project(id, name, path, key_prefix, status, main_branch) VALUES (?1, ?2, ?3, ?4, 'Active', ?5)
         ON CONFLICT(path) DO NOTHING",
        params![p.id.to_string(), p.name, p.path, p.key_prefix, p.main_branch],
    )?;
    if n == 0 {
        return Err(StoreError::Conflict(format!(
            "a project is already registered at {}",
            p.path
        )));
    }
    get(c, p.id)
}

pub fn get(c: &Connection, id: ProjectId) -> Result<Project, StoreError> {
    find(c, id)?.ok_or_else(|| StoreError::NotFound(format!("project {id}")))
}

pub fn find(c: &Connection, id: ProjectId) -> Result<Option<Project>, StoreError> {
    c.query_row(
        &format!("SELECT {COLS} FROM project WHERE id = ?1"),
        [id.to_string()],
        map,
    )
    .optional()?
    .map(decode)
    .transpose()
}

pub fn by_path(c: &Connection, path: &str) -> Result<Option<Project>, StoreError> {
    c.query_row(
        &format!("SELECT {COLS} FROM project WHERE path = ?1"),
        [path],
        map,
    )
    .optional()?
    .map(decode)
    .transpose()
}

pub fn list(c: &Connection) -> Result<Vec<Project>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} FROM project ORDER BY created_at, rowid"
    ))?;
    let rows = st.query_map([], map)?;
    rows.map(|r| decode(r?)).collect()
}

pub fn set_status(c: &Connection, id: ProjectId, status: ProjectStatus) -> Result<(), StoreError> {
    let n = c.execute(
        "UPDATE project SET status = ?2 WHERE id = ?1",
        params![id.to_string(), status.as_str()],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("project {id}")));
    }
    Ok(())
}

pub fn set_settings(
    c: &Connection,
    id: ProjectId,
    settings: &serde_json::Value,
) -> Result<(), StoreError> {
    c.execute(
        "UPDATE project SET settings = ?2 WHERE id = ?1",
        params![id.to_string(), to_json(settings)?],
    )?;
    Ok(())
}

pub fn set_git(c: &Connection, id: ProjectId, git: &serde_json::Value) -> Result<(), StoreError> {
    c.execute(
        "UPDATE project SET git = ?2 WHERE id = ?1",
        params![id.to_string(), to_json(git)?],
    )?;
    Ok(())
}

/// Adds always-allow entries (callers must ensure the user granted them).
pub fn add_allowlist(
    c: &Connection,
    id: ProjectId,
    entries: &[String],
) -> Result<Vec<String>, StoreError> {
    let mut p = get(c, id)?;
    for e in entries {
        if !p.allowlist.contains(e) {
            p.allowlist.push(e.clone());
        }
    }
    c.execute(
        "UPDATE project SET allowlist = ?2 WHERE id = ?1",
        params![id.to_string(), to_json(&p.allowlist)?],
    )?;
    Ok(p.allowlist)
}

/// Allocates the next ticket key (`AGN-<n>`) of the project. Must run inside the write transaction that inserts
/// the ticket so keys stay gap-free on rollback.
pub fn next_ticket_key(c: &Connection, id: ProjectId) -> Result<TicketKey, StoreError> {
    let (prefix, n): (String, i64) = c
        .query_row(
            "UPDATE project SET next_seq = next_seq + 1 WHERE id = ?1 RETURNING key_prefix, next_seq - 1",
            [id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("project {id}")))?;
    TicketKey::new(&prefix, n as u64).map_err(|e| StoreError::Corrupt(e.to_string()))
}
