//! Teams and agents (registry rows; the agent's private memory lives in its own database).

use agenium_core::{AgentId, AgentStatus, BackendKind, ModelBinding, ProjectId, RoleKind, TeamId};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::{from_json, parse, to_json};
use crate::StoreError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Team {
    pub id: TeamId,
    pub name: String,
    pub template: String,
    pub project: Option<ProjectId>,
    pub lead: Option<AgentId>,
    pub solo: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub name: String,
    /// `None`: the agent is in the pool (created, not in a team yet; ADR-61).
    pub team: Option<TeamId>,
    pub role: String,
    pub role_kind: RoleKind,
    pub tags: Vec<String>,
    pub status: AgentStatus,
    pub backend: BackendKind,
    pub model: ModelBinding,
    pub mcp: serde_json::Value,
    pub skills: Vec<String>,
    pub created_at: String,
}

pub fn insert_team(c: &Connection, t: &Team) -> Result<(), StoreError> {
    c.execute(
        "INSERT INTO team(id, name, template, project, lead, solo, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            t.id.to_string(),
            t.name,
            t.template,
            t.project.map(|p| p.to_string()),
            t.lead.map(|a| a.to_string()),
            t.solo,
            t.created_at
        ],
    )?;
    Ok(())
}

type RawTeam = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    bool,
    String,
);

fn map_team(r: &Row<'_>) -> rusqlite::Result<RawTeam> {
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

fn decode_team(v: RawTeam) -> Result<Team, StoreError> {
    let (id, name, template, project, lead, solo, created_at) = v;
    Ok(Team {
        id: parse(&id, "team id")?,
        name,
        template,
        project: project
            .as_deref()
            .map(|p| parse(p, "project id"))
            .transpose()?,
        lead: lead.as_deref().map(|a| parse(a, "agent id")).transpose()?,
        solo,
        created_at,
    })
}

const TEAM_COLS: &str = "id, name, template, project, lead, solo, created_at";

pub fn get_team(c: &Connection, id: TeamId) -> Result<Team, StoreError> {
    c.query_row(
        &format!("SELECT {TEAM_COLS} FROM team WHERE id = ?1"),
        [id.to_string()],
        map_team,
    )
    .optional()?
    .map(decode_team)
    .transpose()?
    .ok_or_else(|| StoreError::NotFound(format!("team {id}")))
}

pub fn teams(c: &Connection) -> Result<Vec<Team>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {TEAM_COLS} FROM team ORDER BY created_at, rowid"
    ))?;
    let rows = st.query_map([], map_team)?;
    rows.map(|r| decode_team(r?)).collect()
}

pub fn teams_of_project(c: &Connection, project: ProjectId) -> Result<Vec<Team>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {TEAM_COLS} FROM team WHERE project = ?1 ORDER BY created_at"
    ))?;
    let rows = st.query_map([project.to_string()], map_team)?;
    rows.map(|r| decode_team(r?)).collect()
}

pub fn set_team_project(
    c: &Connection,
    team: TeamId,
    project: Option<ProjectId>,
) -> Result<(), StoreError> {
    c.execute(
        "UPDATE team SET project = ?2 WHERE id = ?1",
        params![team.to_string(), project.map(|p| p.to_string())],
    )?;
    Ok(())
}

pub fn set_team_lead(
    c: &Connection,
    team: TeamId,
    lead: Option<AgentId>,
) -> Result<(), StoreError> {
    c.execute(
        "UPDATE team SET lead = ?2 WHERE id = ?1",
        params![team.to_string(), lead.map(|a| a.to_string())],
    )?;
    Ok(())
}

pub fn insert_agent(c: &Connection, a: &Agent) -> Result<(), StoreError> {
    c.execute(
        "INSERT INTO agent(id, name, team, role, role_kind, tags, status, backend, model, mcp, skills, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            a.id.to_string(),
            a.name,
            a.team.map(|t| t.to_string()),
            a.role,
            role_kind_str(a.role_kind),
            to_json(&a.tags)?,
            a.status.as_str(),
            backend_str(a.backend),
            to_json(&a.model)?,
            to_json(&a.mcp)?,
            to_json(&a.skills)?,
            a.created_at
        ],
    )
    .map_err(|e| match e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => {
            StoreError::Conflict(format!("an agent named {:?} already exists", a.name))
        }
        e => e.into(),
    })?;
    Ok(())
}

fn role_kind_str(k: RoleKind) -> &'static str {
    match k {
        RoleKind::Lead => "lead",
        RoleKind::Worker => "worker",
    }
}

fn backend_str(b: BackendKind) -> &'static str {
    match b {
        BackendKind::Api => "Api",
        BackendKind::Cli => "Cli",
    }
}

const AGENT_COLS: &str =
    "id, name, team, role, role_kind, tags, status, backend, model, mcp, skills, created_at";

fn map_agent(r: &Row<'_>) -> rusqlite::Result<[String; 12]> {
    let mut v: [String; 12] = Default::default();
    for (i, s) in v.iter_mut().enumerate() {
        // `team` (column 2) is NULL for an agent in the pool.
        *s = r.get::<_, Option<String>>(i)?.unwrap_or_default();
    }
    Ok(v)
}

fn decode_agent(v: [String; 12]) -> Result<Agent, StoreError> {
    let [id, name, team, role, role_kind, tags, status, backend, model, mcp, skills, created_at] =
        v;
    Ok(Agent {
        id: parse(&id, "agent id")?,
        name,
        team: if team.is_empty() {
            None
        } else {
            Some(parse(&team, "team id")?)
        },
        role,
        role_kind: match role_kind.as_str() {
            "lead" => RoleKind::Lead,
            "worker" => RoleKind::Worker,
            other => return Err(StoreError::Corrupt(format!("role_kind {other:?}"))),
        },
        tags: from_json(&tags)?,
        status: parse(&status, "agent status")?,
        backend: match backend.as_str() {
            "Api" => BackendKind::Api,
            "Cli" => BackendKind::Cli,
            other => return Err(StoreError::Corrupt(format!("backend {other:?}"))),
        },
        model: from_json(&model)?,
        mcp: from_json(&mcp)?,
        skills: from_json(&skills)?,
        created_at,
    })
}

pub fn get_agent(c: &Connection, id: AgentId) -> Result<Agent, StoreError> {
    find_agent(c, id)?.ok_or_else(|| StoreError::NotFound(format!("agent {id}")))
}

pub fn find_agent(c: &Connection, id: AgentId) -> Result<Option<Agent>, StoreError> {
    c.query_row(
        &format!("SELECT {AGENT_COLS} FROM agent WHERE id = ?1"),
        [id.to_string()],
        map_agent,
    )
    .optional()?
    .map(decode_agent)
    .transpose()
}

pub fn agent_by_name(c: &Connection, name: &str) -> Result<Option<Agent>, StoreError> {
    c.query_row(
        &format!("SELECT {AGENT_COLS} FROM agent WHERE name = ?1 AND deleted_at IS NULL"),
        [name],
        map_agent,
    )
    .optional()?
    .map(decode_agent)
    .transpose()
}

pub fn agents(c: &Connection) -> Result<Vec<Agent>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {AGENT_COLS} FROM agent WHERE deleted_at IS NULL ORDER BY created_at, rowid"
    ))?;
    let rows = st.query_map([], map_agent)?;
    rows.map(|r| decode_agent(r?)).collect()
}

pub fn agents_of_team(c: &Connection, team: TeamId) -> Result<Vec<Agent>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {AGENT_COLS} FROM agent WHERE team = ?1 AND deleted_at IS NULL ORDER BY created_at, rowid"
    ))?;
    let rows = st.query_map([team.to_string()], map_agent)?;
    rows.map(|r| decode_agent(r?)).collect()
}

/// Sets the status and returns the previous one.
pub fn set_agent_status(
    c: &Connection,
    id: AgentId,
    to: AgentStatus,
) -> Result<AgentStatus, StoreError> {
    let prev = get_agent(c, id)?.status;
    c.execute(
        "UPDATE agent SET status = ?2 WHERE id = ?1",
        params![id.to_string(), to.as_str()],
    )?;
    Ok(prev)
}

/// Moves an agent into a team (`Some`) or back to the pool (`None`).
pub fn set_agent_team(c: &Connection, id: AgentId, team: Option<TeamId>) -> Result<(), StoreError> {
    let n = c.execute(
        "UPDATE agent SET team = ?2 WHERE id = ?1 AND deleted_at IS NULL",
        params![id.to_string(), team.map(|t| t.to_string())],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("agent {id}")));
    }
    Ok(())
}

pub fn set_agent_role(
    c: &Connection,
    id: AgentId,
    role: &str,
    tags: &[String],
) -> Result<(), StoreError> {
    let n = c.execute(
        "UPDATE agent SET role = ?2, tags = ?3 WHERE id = ?1 AND deleted_at IS NULL",
        params![id.to_string(), role, to_json(&tags)?],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("agent {id}")));
    }
    Ok(())
}

pub fn delete_team(c: &Connection, id: TeamId) -> Result<(), StoreError> {
    c.execute("DELETE FROM team WHERE id = ?1", [id.to_string()])?;
    Ok(())
}

pub fn set_agent_role_kind(c: &Connection, id: AgentId, kind: RoleKind) -> Result<(), StoreError> {
    c.execute(
        "UPDATE agent SET role_kind = ?2 WHERE id = ?1",
        params![
            id.to_string(),
            match kind {
                RoleKind::Lead => "lead",
                RoleKind::Worker => "worker",
            }
        ],
    )?;
    Ok(())
}

pub fn set_agent_model(
    c: &Connection,
    id: AgentId,
    model: &ModelBinding,
) -> Result<(), StoreError> {
    let n = c.execute(
        "UPDATE agent SET model = ?2 WHERE id = ?1",
        params![id.to_string(), to_json(model)?],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("agent {id}")));
    }
    Ok(())
}

pub fn set_agent_backend(
    c: &Connection,
    id: AgentId,
    backend: BackendKind,
) -> Result<(), StoreError> {
    let n = c.execute(
        "UPDATE agent SET backend = ?2 WHERE id = ?1",
        params![id.to_string(), backend_str(backend)],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("agent {id}")));
    }
    Ok(())
}

pub fn set_agent_skills(c: &Connection, id: AgentId, skills: &[String]) -> Result<(), StoreError> {
    let n = c.execute(
        "UPDATE agent SET skills = ?2 WHERE id = ?1",
        params![id.to_string(), to_json(&skills)?],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("agent {id}")));
    }
    Ok(())
}

/// Turns the agent into a tombstone (decision 32): renamed `<name>~<id>`, Offline, binding cleared, hidden from
/// listings. References from runs, tickets and events stay valid.
pub fn tombstone_agent(c: &Connection, id: AgentId) -> Result<(), StoreError> {
    let n = c.execute(
        "UPDATE agent SET name = name || '~' || id, status = 'Offline', mcp = '[]', skills = '[]',
                deleted_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = ?1 AND deleted_at IS NULL",
        [id.to_string()],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("agent {id}")));
    }
    Ok(())
}

pub fn set_agent_mcp(
    c: &Connection,
    id: AgentId,
    mcp: &serde_json::Value,
) -> Result<(), StoreError> {
    let n = c.execute(
        "UPDATE agent SET mcp = ?2 WHERE id = ?1",
        params![id.to_string(), mcp.to_string()],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("agent {id}")));
    }
    Ok(())
}

pub fn deleted_agents(c: &Connection) -> Result<Vec<AgentId>, StoreError> {
    let mut st = c.prepare_cached("SELECT id FROM agent WHERE deleted_at IS NOT NULL")?;
    let ids: Vec<String> = st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    ids.iter().map(|s| parse(s, "agent id")).collect()
}

pub fn is_deleted(c: &Connection, id: AgentId) -> Result<bool, StoreError> {
    Ok(c.query_row(
        "SELECT deleted_at IS NOT NULL FROM agent WHERE id = ?1",
        [id.to_string()],
        |r| r.get::<_, bool>(0),
    )
    .optional()?
    .unwrap_or(false))
}
