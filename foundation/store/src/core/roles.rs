//! Role proposals of a Lead (decision 3l): pending until the user approves (an agent is created) or declines.

use agenium_core::forms::ProposeRole;
use agenium_core::{AgentId, ProjectId, TeamId};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::{from_json, parse, to_json};
use crate::StoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProposalStatus {
    Pending,
    Approved,
    Declined,
}

impl ProposalStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Declined => "declined",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleProposal {
    pub id: String,
    pub project: ProjectId,
    pub team: TeamId,
    pub proposed_by: AgentId,
    pub body: ProposeRole,
    pub status: ProposalStatus,
    pub agent: Option<AgentId>,
    pub note: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

const COLS: &str =
    "id, project, team, proposed_by, body, status, agent, note, created_at, decided_at";

fn row(r: &Row) -> rusqlite::Result<[Option<String>; 10]> {
    Ok([
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
    ])
}

fn decode(v: [Option<String>; 10]) -> Result<RoleProposal, StoreError> {
    let [id, project, team, by, body, status, agent, note, created, decided] = v;
    let s = |o: Option<String>| o.unwrap_or_default();
    Ok(RoleProposal {
        id: s(id),
        project: parse(&s(project), "project id")?,
        team: parse(&s(team), "team id")?,
        proposed_by: parse(&s(by), "agent id")?,
        body: from_json(&s(body))?,
        status: match s(status).as_str() {
            "approved" => ProposalStatus::Approved,
            "declined" => ProposalStatus::Declined,
            _ => ProposalStatus::Pending,
        },
        agent: agent.map(|a| parse(&a, "agent id")).transpose()?,
        note,
        created_at: s(created),
        decided_at: decided,
    })
}

pub fn insert(c: &Connection, p: &RoleProposal) -> Result<(), StoreError> {
    c.execute(
        &format!(
            "INSERT INTO role_proposal({COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
        ),
        params![
            p.id,
            p.project.to_string(),
            p.team.to_string(),
            p.proposed_by.to_string(),
            to_json(&p.body)?,
            p.status.as_str(),
            p.agent.map(|a| a.to_string()),
            p.note,
            p.created_at,
            p.decided_at
        ],
    )?;
    Ok(())
}

pub fn get(c: &Connection, id: &str) -> Result<Option<RoleProposal>, StoreError> {
    c.query_row(
        &format!("SELECT {COLS} FROM role_proposal WHERE id = ?1"),
        params![id],
        row,
    )
    .optional()?
    .map(decode)
    .transpose()
}

/// The project's proposals, oldest first.
pub fn for_project(c: &Connection, project: ProjectId) -> Result<Vec<RoleProposal>, StoreError> {
    let mut st = c.prepare(&format!(
        "SELECT {COLS} FROM role_proposal WHERE project = ?1 ORDER BY created_at, id"
    ))?;
    let rows = st
        .query_map(params![project.to_string()], row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter().map(decode).collect()
}

/// Records the user's decision on a pending proposal.
pub fn decide(
    c: &Connection,
    id: &str,
    status: ProposalStatus,
    agent: Option<AgentId>,
    note: Option<&str>,
    at: &str,
) -> Result<(), StoreError> {
    let n = c.execute(
        "UPDATE role_proposal SET status = ?2, agent = ?3, note = ?4, decided_at = ?5 WHERE id = ?1 AND status = 'pending'",
        params![id, status.as_str(), agent.map(|a| a.to_string()), note, at],
    )?;
    if n == 0 {
        return Err(StoreError::Conflict(format!(
            "role proposal {id} does not exist or was already decided"
        )));
    }
    Ok(())
}
