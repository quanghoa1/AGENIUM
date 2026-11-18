//! Removing a project that never started from Agenium (the user's button on its card). Only an empty one: no team, no
//! ticket, spec or plan, no message, proposal or permission. Its record goes; its folder and everything in it stay as
//! they are (creating a project there again opens it as it is: `agenium.toml` is kept). A project with work is closed
//! instead (ADR-130), which keeps its history.

use agenium_core::{DomainEvent, ProjectId};
use agenium_store::core::projects;
use rusqlite::params;

use crate::{App, OrchError, Result};

/// What makes a project not empty, by table: the rows that name it.
const HELD_BY: [(&str, &str); 8] = [
    ("team", "a team"),
    ("ticket", "tickets"),
    ("spec", "a spec"),
    ("plan", "a plan"),
    ("lead_message", "messages to its Lead"),
    ("role_proposal", "role proposals"),
    ("skill_proposal", "skill proposals"),
    ("permission_grant", "permissions"),
];

impl App {
    /// Removes the empty project `project` from Agenium; its folder stays untouched.
    pub fn remove_project(&self, project: ProjectId) -> Result<()> {
        let id = project.to_string();
        self.store().commit(None, "user", |w| -> Result<()> {
            let p = projects::get(w.tx, project)?;
            let held: Vec<&str> = HELD_BY
                .iter()
                .filter(|(table, _)| {
                    w.tx.query_row(
                        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE project = ?1)"),
                        [&id],
                        |r| r.get::<_, bool>(0),
                    )
                    .unwrap_or(true)
                })
                .map(|(_, what)| *what)
                .collect();
            if !held.is_empty() {
                return Err(OrchError::Conflict(format!(
                    "{} has {}: only an empty project is removed; close it instead (its history stays)",
                    p.name,
                    held.join(", ")
                )));
            }
            for table in ["map_link", "map_node"] {
                w.tx.execute(
                    &format!("DELETE FROM {table} WHERE project = ?1"),
                    params![id],
                )
                .map_err(agenium_store::StoreError::from)?;
            }
            w.tx.execute("DELETE FROM project WHERE id = ?1", params![id])
                .map_err(agenium_store::StoreError::from)?;
            w.emit(DomainEvent::ProjectRemoved {
                project,
                name: p.name.clone(),
                path: p.path.clone(),
            })?;
            Ok(())
        })?;
        Ok(())
    }
}
