//! New skills agents propose (ADR-91 amended by ADR-152): pending until the user saves one into the library or declines.

use agenium_core::{AgentId, ProjectId};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::roles::ProposalStatus;
use crate::StoreError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillProposal {
    pub id: String,
    pub project: Option<ProjectId>,
    pub agent: AgentId,
    pub name: String,
    /// The SKILL.md it proposes.
    pub text: String,
    pub lesson: i64,
    pub lesson_text: String,
    /// Where the lesson held (`<project>/ticket:<KEY>`), the evidence the user reads.
    pub contexts: Vec<String>,
    /// Library skills close to it (name, scores, level), for a merge.
    pub similar: serde_json::Value,
    /// Where it contradicts a library skill the agent does not have.
    pub notes: Vec<String>,
    pub reason: String,
    pub status: ProposalStatus,
    /// The library skill it was saved as.
    pub skill: Option<String>,
    pub note: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

const COLS: &str =
    "id, project, agent, name, text, lesson, lesson_text, reason, status, skill, note, created_at, decided_at, contexts, similar, notes";

fn map(r: &Row) -> rusqlite::Result<SkillProposal> {
    let id = |s: Option<String>| s.and_then(|s| s.parse().ok());
    let status: String = r.get(8)?;
    Ok(SkillProposal {
        id: r.get(0)?,
        project: id(r.get(1)?),
        agent: r.get::<_, String>(2)?.parse().map_err(|_| {
            rusqlite::Error::InvalidColumnType(2, "agent".into(), rusqlite::types::Type::Text)
        })?,
        name: r.get(3)?,
        text: r.get(4)?,
        lesson: r.get(5)?,
        lesson_text: r.get(6)?,
        reason: r.get(7)?,
        status: match status.as_str() {
            "approved" => ProposalStatus::Approved,
            "declined" => ProposalStatus::Declined,
            _ => ProposalStatus::Pending,
        },
        skill: r.get(9)?,
        note: r.get(10)?,
        created_at: r.get(11)?,
        decided_at: r.get(12)?,
        contexts: serde_json::from_str(&r.get::<_, String>(13)?).unwrap_or_default(),
        similar: serde_json::from_str(&r.get::<_, String>(14)?).unwrap_or_default(),
        notes: serde_json::from_str(&r.get::<_, String>(15)?).unwrap_or_default(),
    })
}

pub fn insert(c: &Connection, p: &SkillProposal) -> Result<(), StoreError> {
    c.execute(
        &format!("INSERT INTO skill_proposal ({COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending', NULL, NULL, ?9, NULL, ?10, ?11, ?12)"),
        params![
            p.id,
            p.project.map(|x| x.to_string()),
            p.agent.to_string(),
            p.name,
            p.text,
            p.lesson,
            p.lesson_text,
            p.reason,
            p.created_at,
            serde_json::to_string(&p.contexts).unwrap_or_else(|_| "[]".into()),
            p.similar.to_string(),
            serde_json::to_string(&p.notes).unwrap_or_else(|_| "[]".into())
        ],
    )?;
    Ok(())
}

pub fn get(c: &Connection, id: &str) -> Result<Option<SkillProposal>, StoreError> {
    Ok(c.query_row(
        &format!("SELECT {COLS} FROM skill_proposal WHERE id = ?1"),
        params![id],
        map,
    )
    .optional()?)
}

/// The pending proposals, of `project` or of every project (oldest first).
pub fn pending(
    c: &Connection,
    project: Option<ProjectId>,
) -> Result<Vec<SkillProposal>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} FROM skill_proposal WHERE status = 'pending' AND (?1 IS NULL OR project = ?1) ORDER BY created_at"
    ))?;
    let rows = st.query_map(params![project.map(|p| p.to_string())], map)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// An agent's proposals that are pending or were saved (the lessons and names they hold).
pub fn of_agent(c: &Connection, agent: AgentId) -> Result<Vec<SkillProposal>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} FROM skill_proposal WHERE agent = ?1 AND status != 'declined' ORDER BY created_at"
    ))?;
    let rows = st.query_map(params![agent.to_string()], map)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Records the user's decision (or a proposal withdrawn because its lesson stopped holding: declined with why).
pub fn decide(
    c: &Connection,
    id: &str,
    status: ProposalStatus,
    skill: Option<&str>,
    note: Option<&str>,
    at: &str,
) -> Result<(), StoreError> {
    let s = match status {
        ProposalStatus::Approved => "approved",
        ProposalStatus::Declined => "declined",
        ProposalStatus::Pending => "pending",
    };
    c.execute(
        "UPDATE skill_proposal SET status = ?2, skill = ?3, note = ?4, decided_at = ?5 WHERE id = ?1 AND status = 'pending'",
        params![id, s, skill, note, at],
    )?;
    Ok(())
}
