//! Ticket workspaces: worktree path, branch and the shas the merge flow and reconcile rely on.

use agenium_core::TicketId;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::parse;
use crate::StoreError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketWorkspace {
    pub ticket: TicketId,
    pub worktree: String,
    pub branch: String,
    pub base_sha: String,
    pub head_sha: Option<String>,
    pub merged_sha: Option<String>,
    pub removed: bool,
}

pub fn upsert(c: &Connection, w: &TicketWorkspace) -> Result<(), StoreError> {
    c.execute(
        "INSERT INTO ticket_workspace(ticket, worktree, branch, base_sha, head_sha, merged_sha, removed)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(ticket) DO UPDATE SET worktree = excluded.worktree, branch = excluded.branch,
             base_sha = excluded.base_sha, head_sha = excluded.head_sha, merged_sha = excluded.merged_sha,
             removed = excluded.removed",
        params![w.ticket.to_string(), w.worktree, w.branch, w.base_sha, w.head_sha, w.merged_sha, w.removed],
    )?;
    Ok(())
}

pub fn get(c: &Connection, ticket: TicketId) -> Result<Option<TicketWorkspace>, StoreError> {
    let raw = c
        .query_row(
            "SELECT ticket, worktree, branch, base_sha, head_sha, merged_sha, removed FROM ticket_workspace WHERE ticket = ?1",
            [ticket.to_string()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .optional()?;
    raw.map(
        |(t, worktree, branch, base_sha, head_sha, merged_sha, removed)| {
            Ok(TicketWorkspace {
                ticket: parse(&t, "ticket id")?,
                worktree,
                branch,
                base_sha,
                head_sha,
                merged_sha,
                removed,
            })
        },
    )
    .transpose()
}

/// Workspaces of a project that still have a worktree on disk (not removed).
pub fn live_for_project(
    c: &Connection,
    project: agenium_core::ProjectId,
) -> Result<Vec<TicketWorkspace>, StoreError> {
    let mut st = c.prepare_cached(
        "SELECT w.ticket FROM ticket_workspace w JOIN ticket t ON t.id = w.ticket WHERE t.project = ?1 AND w.removed = 0",
    )?;
    let ids: Vec<String> = st
        .query_map([project.to_string()], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for id in ids {
        if let Some(w) = get(c, parse(&id, "ticket id")?)? {
            out.push(w);
        }
    }
    Ok(out)
}
