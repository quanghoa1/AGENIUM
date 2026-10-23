//! Tickets and their dependency edges. Domain rules (state machine, DAG check, plan item) are enforced by the
//! orchestrator before calling these; the store only keeps rows consistent.

use std::collections::HashMap;

use agenium_core::{
    AcceptanceCriterion, AgentId, ProjectId, Ticket, TicketId, TicketKey, TicketKind, TicketStatus,
    UsageSummary,
};
use rusqlite::{params, Connection, OptionalExtension, Row};

use super::{from_json, parse, to_json};
use crate::StoreError;

const COLS: &str = "t.id, t.key, t.project, p.key, t.kind, t.title, t.description, t.acceptance, t.status, t.assignee, \
    t.plan_item, t.priority, t.reopen_count, t.escalated, t.required_role, t.required_tags, t.scope_paths, \
    t.report_template, t.approved, t.merged, t.has_code, t.close_reason, t.usage, t.created_at, t.updated_at";
const FROM: &str = "FROM ticket t LEFT JOIN ticket p ON p.id = t.parent";

struct RawTicket {
    id: String,
    key: String,
    project: String,
    parent_key: Option<String>,
    kind: String,
    title: String,
    description: String,
    acceptance: String,
    status: String,
    assignee: Option<String>,
    plan_item: String,
    priority: i64,
    reopen_count: i64,
    escalated: bool,
    required_role: Option<String>,
    required_tags: String,
    scope_paths: String,
    report_template: String,
    approved: bool,
    merged: bool,
    has_code: bool,
    close_reason: Option<String>,
    usage: String,
    created_at: String,
    updated_at: String,
}

fn map(r: &Row<'_>) -> rusqlite::Result<RawTicket> {
    Ok(RawTicket {
        id: r.get(0)?,
        key: r.get(1)?,
        project: r.get(2)?,
        parent_key: r.get(3)?,
        kind: r.get(4)?,
        title: r.get(5)?,
        description: r.get(6)?,
        acceptance: r.get(7)?,
        status: r.get(8)?,
        assignee: r.get(9)?,
        plan_item: r.get(10)?,
        priority: r.get(11)?,
        reopen_count: r.get(12)?,
        escalated: r.get(13)?,
        required_role: r.get(14)?,
        required_tags: r.get(15)?,
        scope_paths: r.get(16)?,
        report_template: r.get(17)?,
        approved: r.get(18)?,
        merged: r.get(19)?,
        has_code: r.get(20)?,
        close_reason: r.get(21)?,
        usage: r.get(22)?,
        created_at: r.get(23)?,
        updated_at: r.get(24)?,
    })
}

fn decode(c: &Connection, r: RawTicket) -> Result<Ticket, StoreError> {
    let id: TicketId = parse(&r.id, "ticket id")?;
    Ok(Ticket {
        id,
        key: parse(&r.key, "ticket key")?,
        project: parse(&r.project, "project id")?,
        parent: r
            .parent_key
            .as_deref()
            .map(|k| parse(k, "ticket key"))
            .transpose()?,
        kind: parse(&r.kind, "ticket kind")?,
        title: r.title,
        description: r.description,
        acceptance: from_json::<Vec<AcceptanceCriterion>>(&r.acceptance)?,
        status: parse(&r.status, "ticket status")?,
        assignee: r
            .assignee
            .as_deref()
            .map(|a| parse(a, "agent id"))
            .transpose()?,
        plan_item: r.plan_item,
        priority: r.priority as u8,
        depends_on: dep_keys(c, id)?,
        reopen_count: r.reopen_count as u32,
        escalated: r.escalated,
        required_role: r.required_role,
        required_tags: from_json(&r.required_tags)?,
        scope_paths: from_json(&r.scope_paths)?,
        report_template: r.report_template,
        approved: r.approved,
        merged: r.merged,
        has_code: r.has_code,
        close_reason: r.close_reason,
        usage: from_json::<UsageSummary>(&r.usage)?,
        created_at: r.created_at,
        updated_at: r.updated_at,
    })
}

fn dep_keys(c: &Connection, id: TicketId) -> Result<Vec<TicketKey>, StoreError> {
    let mut st = c.prepare_cached(
        "SELECT d.key FROM ticket_dep td JOIN ticket d ON d.id = td.depends_on WHERE td.ticket = ?1 ORDER BY td.pos",
    )?;
    let rows = st.query_map([id.to_string()], |r| r.get::<_, String>(0))?;
    rows.map(|k| parse(&k?, "ticket key")).collect()
}

/// Fields of a ticket that does not exist yet (key allocated by [`super::projects::next_ticket_key`]).
#[derive(Debug, Clone)]
pub struct TicketInsert {
    pub id: TicketId,
    pub key: TicketKey,
    pub project: ProjectId,
    pub parent: Option<TicketId>,
    pub kind: TicketKind,
    pub title: String,
    pub description: String,
    pub acceptance: Vec<AcceptanceCriterion>,
    pub plan_item: String,
    pub priority: u8,
    pub depends_on: Vec<TicketId>,
    pub required_role: Option<String>,
    pub required_tags: Vec<String>,
    pub scope_paths: Vec<String>,
    pub report_template: String,
    pub created_at: String,
}

pub fn insert(c: &Connection, t: &TicketInsert) -> Result<Ticket, StoreError> {
    c.execute(
        "INSERT INTO ticket(id, key, project, parent, kind, title, description, acceptance, status, plan_item, priority,
                            required_role, required_tags, scope_paths, report_template, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'New', ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
        params![
            t.id.to_string(),
            t.key.as_str(),
            t.project.to_string(),
            t.parent.map(|p| p.to_string()),
            t.kind.as_str(),
            t.title,
            t.description,
            to_json(&t.acceptance)?,
            t.plan_item,
            t.priority as i64,
            t.required_role,
            to_json(&t.required_tags)?,
            to_json(&t.scope_paths)?,
            t.report_template,
            t.created_at,
        ],
    )?;
    set_deps(c, t.id, &t.depends_on)?;
    get(c, t.id)
}

/// Replaces the dependency list (cycle checks are the caller's job).
pub fn set_deps(c: &Connection, id: TicketId, deps: &[TicketId]) -> Result<(), StoreError> {
    c.execute("DELETE FROM ticket_dep WHERE ticket = ?1", [id.to_string()])?;
    for (pos, d) in deps.iter().enumerate() {
        c.execute(
            "INSERT INTO ticket_dep(ticket, depends_on, pos) VALUES (?1, ?2, ?3)",
            params![id.to_string(), d.to_string(), pos as i64],
        )?;
    }
    Ok(())
}

/// Persists every mutable field of `t` (status, flags, assignment, text fields, usage) and bumps `updated_at`.
pub fn update(c: &Connection, t: &Ticket) -> Result<(), StoreError> {
    let n = c.execute(
        "UPDATE ticket SET title = ?2, description = ?3, acceptance = ?4, status = ?5, assignee = ?6, priority = ?7,
             reopen_count = ?8, escalated = ?9, required_role = ?10, required_tags = ?11, scope_paths = ?12,
             report_template = ?13, approved = ?14, merged = ?15, has_code = ?16, close_reason = ?17, usage = ?18,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = ?1",
        params![
            t.id.to_string(),
            t.title,
            t.description,
            to_json(&t.acceptance)?,
            t.status.as_str(),
            t.assignee.map(|a| a.to_string()),
            t.priority as i64,
            t.reopen_count as i64,
            t.escalated,
            t.required_role,
            to_json(&t.required_tags)?,
            to_json(&t.scope_paths)?,
            t.report_template,
            t.approved,
            t.merged,
            t.has_code,
            t.close_reason,
            to_json(&t.usage)?,
        ],
    )?;
    if n == 0 {
        return Err(StoreError::NotFound(format!("ticket {}", t.key)));
    }
    Ok(())
}

pub fn get(c: &Connection, id: TicketId) -> Result<Ticket, StoreError> {
    find(c, id)?.ok_or_else(|| StoreError::NotFound(format!("ticket {id}")))
}

pub fn find(c: &Connection, id: TicketId) -> Result<Option<Ticket>, StoreError> {
    let raw = c
        .query_row(
            &format!("SELECT {COLS} {FROM} WHERE t.id = ?1"),
            [id.to_string()],
            map,
        )
        .optional()?;
    raw.map(|r| decode(c, r)).transpose()
}

pub fn by_key(
    c: &Connection,
    project: ProjectId,
    key: &TicketKey,
) -> Result<Option<Ticket>, StoreError> {
    let raw = c
        .query_row(
            &format!("SELECT {COLS} {FROM} WHERE t.project = ?1 AND t.key = ?2"),
            params![project.to_string(), key.as_str()],
            map,
        )
        .optional()?;
    raw.map(|r| decode(c, r)).transpose()
}

pub fn get_by_key(
    c: &Connection,
    project: ProjectId,
    key: &TicketKey,
) -> Result<Ticket, StoreError> {
    by_key(c, project, key)?.ok_or_else(|| StoreError::NotFound(format!("ticket {key}")))
}

/// Filter for [`list`].
#[derive(Debug, Clone, Default)]
pub struct TicketFilter {
    pub statuses: Vec<TicketStatus>,
    pub assignee: Option<AgentId>,
    pub plan_item: Option<String>,
}

pub fn list(
    c: &Connection,
    project: ProjectId,
    f: &TicketFilter,
) -> Result<Vec<Ticket>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} {FROM} WHERE t.project = ?1 ORDER BY t.created_at, t.rowid"
    ))?;
    let raws: Vec<RawTicket> = st
        .query_map([project.to_string()], map)?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for r in raws {
        if !f.statuses.is_empty() && !f.statuses.iter().any(|s| s.as_str() == r.status) {
            continue;
        }
        if let Some(a) = f.assignee {
            if r.assignee.as_deref() != Some(a.to_string().as_str()) {
                continue;
            }
        }
        if let Some(pi) = &f.plan_item {
            if &r.plan_item != pi {
                continue;
            }
        }
        out.push(decode(c, r)?);
    }
    Ok(out)
}

/// Tickets assigned to `agent` that are not terminal.
pub fn open_for_agent(c: &Connection, agent: AgentId) -> Result<Vec<Ticket>, StoreError> {
    let mut st = c.prepare_cached(&format!(
        "SELECT {COLS} {FROM} WHERE t.assignee = ?1 AND t.status NOT IN ('Closed','Cancelled') ORDER BY t.priority, t.created_at"
    ))?;
    let raws: Vec<RawTicket> = st
        .query_map([agent.to_string()], map)?
        .collect::<Result<_, _>>()?;
    raws.into_iter().map(|r| decode(c, r)).collect()
}

/// Ids of tickets that depend on `id`.
pub fn dependents(c: &Connection, id: TicketId) -> Result<Vec<TicketId>, StoreError> {
    let mut st = c.prepare_cached("SELECT ticket FROM ticket_dep WHERE depends_on = ?1")?;
    let rows = st.query_map([id.to_string()], |r| r.get::<_, String>(0))?;
    rows.map(|s| parse(&s?, "ticket id")).collect()
}

/// The whole dependency graph of a project: ticket id -> its dependency ids.
pub fn dep_graph(
    c: &Connection,
    project: ProjectId,
) -> Result<HashMap<TicketId, Vec<TicketId>>, StoreError> {
    let mut st = c.prepare_cached(
        "SELECT td.ticket, td.depends_on FROM ticket_dep td JOIN ticket t ON t.id = td.ticket WHERE t.project = ?1 ORDER BY td.pos",
    )?;
    let rows = st.query_map([project.to_string()], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut g: HashMap<TicketId, Vec<TicketId>> = HashMap::new();
    for row in rows {
        let (a, b) = row?;
        g.entry(parse(&a, "ticket id")?)
            .or_default()
            .push(parse(&b, "ticket id")?);
    }
    Ok(g)
}

pub fn add_link(
    c: &Connection,
    src: TicketId,
    dst: TicketId,
    kind: &str,
    proposed_by: &str,
    confirmed_by: Option<&str>,
    note: Option<&str>,
) -> Result<(), StoreError> {
    c.execute(
        "INSERT INTO ticket_link(src, dst, kind, proposed_by, confirmed_by, note) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(src, dst, kind) DO UPDATE SET confirmed_by = COALESCE(excluded.confirmed_by, confirmed_by)",
        params![src.to_string(), dst.to_string(), kind, proposed_by, confirmed_by, note],
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Link {
    pub src: TicketId,
    pub dst: TicketId,
    pub kind: String,
    pub proposed_by: String,
    pub confirmed_by: Option<String>,
    pub note: Option<String>,
}

pub fn links_of(c: &Connection, id: TicketId) -> Result<Vec<Link>, StoreError> {
    let mut st = c.prepare_cached(
        "SELECT src, dst, kind, proposed_by, confirmed_by, note FROM ticket_link WHERE src = ?1 OR dst = ?1 ORDER BY kind",
    )?;
    let rows = st.query_map([id.to_string()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        ))
    })?;
    rows.map(|r| {
        let (s, d, kind, proposed_by, confirmed_by, note) = r?;
        Ok(Link {
            src: parse(&s, "ticket id")?,
            dst: parse(&d, "ticket id")?,
            kind,
            proposed_by,
            confirmed_by,
            note,
        })
    })
    .collect()
}
