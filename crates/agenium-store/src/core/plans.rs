//! Requirement specs and plans, versioned per project (decision 3h).

use agenium_core::plan::{RequirementSpec, TicketPlan};
use agenium_core::ProjectId;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{from_json, parse, to_json};
use crate::StoreError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecRow {
    pub id: String,
    pub project: ProjectId,
    pub version: u32,
    pub body: RequirementSpec,
    pub proposed_by: String,
    pub approved_by: Option<String>,
    pub locked_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanRow {
    pub id: String,
    pub project: ProjectId,
    pub version: u32,
    pub spec_version: u32,
    pub reason: Option<String>,
    pub diff: Option<String>,
    pub body: TicketPlan,
    pub proposed_by: String,
    pub approved_by: Option<String>,
}

pub fn insert_spec(c: &Connection, s: &SpecRow) -> Result<(), StoreError> {
    c.execute(
        "INSERT INTO spec(id, project, version, body, proposed_by, approved_by, locked_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![s.id, s.project.to_string(), s.version as i64, to_json(&s.body)?, s.proposed_by, s.approved_by, s.locked_at],
    )?;
    Ok(())
}

pub fn approve_spec(
    c: &Connection,
    project: ProjectId,
    version: u32,
    by: &str,
    at: &str,
) -> Result<(), StoreError> {
    let n = c.execute(
        "UPDATE spec SET approved_by = ?3, locked_at = ?4 WHERE project = ?1 AND version = ?2 AND approved_by IS NULL",
        params![project.to_string(), version as i64, by, at],
    )?;
    if n == 0 {
        return Err(StoreError::Conflict(format!(
            "spec v{version} does not exist or is already approved"
        )));
    }
    Ok(())
}

fn spec_query(
    c: &Connection,
    sql: &str,
    p: impl rusqlite::Params,
) -> Result<Option<SpecRow>, StoreError> {
    let raw = c
        .query_row(sql, p, |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })
        .optional()?;
    raw.map(
        |(id, project, version, body, proposed_by, approved_by, locked_at)| {
            Ok(SpecRow {
                id,
                project: parse(&project, "project id")?,
                version: version as u32,
                body: from_json(&body)?,
                proposed_by,
                approved_by,
                locked_at,
            })
        },
    )
    .transpose()
}

/// Latest spec version of a project (approved or not).
pub fn latest_spec(c: &Connection, project: ProjectId) -> Result<Option<SpecRow>, StoreError> {
    spec_query(
        c,
        "SELECT id, project, version, body, proposed_by, approved_by, locked_at FROM spec WHERE project = ?1 ORDER BY version DESC LIMIT 1",
        [project.to_string()],
    )
}

pub fn spec(
    c: &Connection,
    project: ProjectId,
    version: u32,
) -> Result<Option<SpecRow>, StoreError> {
    spec_query(
        c,
        "SELECT id, project, version, body, proposed_by, approved_by, locked_at FROM spec WHERE project = ?1 AND version = ?2",
        params![project.to_string(), version as i64],
    )
}

pub fn insert_plan(c: &Connection, p: &PlanRow) -> Result<(), StoreError> {
    c.execute(
        "INSERT INTO plan(id, project, version, spec_version, reason, diff, body, proposed_by, approved_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            p.id,
            p.project.to_string(),
            p.version as i64,
            p.spec_version as i64,
            p.reason,
            p.diff,
            to_json(&p.body)?,
            p.proposed_by,
            p.approved_by
        ],
    )?;
    Ok(())
}

pub fn approve_plan(
    c: &Connection,
    project: ProjectId,
    version: u32,
    by: &str,
) -> Result<(), StoreError> {
    let n = c.execute(
        "UPDATE plan SET approved_by = ?3 WHERE project = ?1 AND version = ?2 AND approved_by IS NULL",
        params![project.to_string(), version as i64, by],
    )?;
    if n == 0 {
        return Err(StoreError::Conflict(format!(
            "plan v{version} does not exist or is already approved"
        )));
    }
    Ok(())
}

fn plan_query(
    c: &Connection,
    sql: &str,
    p: impl rusqlite::Params,
) -> Result<Option<PlanRow>, StoreError> {
    let raw = c
        .query_row(sql, p, |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        })
        .optional()?;
    raw.map(
        |(id, project, version, spec_version, reason, diff, body, proposed_by, approved_by)| {
            Ok(PlanRow {
                id,
                project: parse(&project, "project id")?,
                version: version as u32,
                spec_version: spec_version as u32,
                reason,
                diff,
                body: from_json(&body)?,
                proposed_by,
                approved_by,
            })
        },
    )
    .transpose()
}

const PLAN_COLS: &str =
    "id, project, version, spec_version, reason, diff, body, proposed_by, approved_by";

/// Latest plan version (approved or pending).
pub fn latest_plan(c: &Connection, project: ProjectId) -> Result<Option<PlanRow>, StoreError> {
    plan_query(
        c,
        &format!("SELECT {PLAN_COLS} FROM plan WHERE project = ?1 ORDER BY version DESC LIMIT 1"),
        [project.to_string()],
    )
}

/// Latest approved plan: tickets may only reference its items.
pub fn current_plan(c: &Connection, project: ProjectId) -> Result<Option<PlanRow>, StoreError> {
    plan_query(
        c,
        &format!("SELECT {PLAN_COLS} FROM plan WHERE project = ?1 AND approved_by IS NOT NULL ORDER BY version DESC LIMIT 1"),
        [project.to_string()],
    )
}

pub fn plan(
    c: &Connection,
    project: ProjectId,
    version: u32,
) -> Result<Option<PlanRow>, StoreError> {
    plan_query(
        c,
        &format!("SELECT {PLAN_COLS} FROM plan WHERE project = ?1 AND version = ?2"),
        params![project.to_string(), version as i64],
    )
}
