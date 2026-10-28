//! Skill overlays the agent learned (ADR-88).

use super::*;

impl AgentStore {
    /// Records an update of the agent's overlay of `skill` (decision 35): the base version it starts from, the
    /// cumulative patch base -> overlay, and one changelog entry appended.
    pub fn put_overlay(
        &self,
        skill: &str,
        base_version: &str,
        patch: &str,
        entry: serde_json::Value,
    ) -> Result<(), StoreError> {
        self.db.write(|tx| {
            let old: Option<String> = tx
                .query_row("SELECT changelog FROM skill_overlay WHERE skill = ?1", [skill], |r| r.get(0))
                .optional()?;
            let mut log: Vec<serde_json::Value> =
                old.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            log.push(entry);
            tx.execute(
                "INSERT INTO skill_overlay(skill, base_version, patch, changelog) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(skill) DO UPDATE SET base_version = excluded.base_version, patch = excluded.patch,
                     changelog = excluded.changelog",
                params![skill, base_version, patch, serde_json::to_string(&log).unwrap_or_else(|_| "[]".into())],
            )?;
            Ok(())
        })
    }

    /// The agent's skill overlays.
    pub fn overlays(&self) -> Result<Vec<OverlayRow>, StoreError> {
        self.db.read(|c| {
            let mut st = c.prepare(
                "SELECT skill, base_version, patch, changelog FROM skill_overlay ORDER BY skill",
            )?;
            let rows = st
                .query_map([], |r| {
                    let log: String = r.get(3)?;
                    Ok(OverlayRow {
                        skill: r.get(0)?,
                        base_version: r.get(1)?,
                        patch: r.get(2)?,
                        changelog: serde_json::from_str(&log).unwrap_or_default(),
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn remove_overlay(&self, skill: &str) -> Result<(), StoreError> {
        self.db.write(|tx| {
            tx.execute("DELETE FROM skill_overlay WHERE skill = ?1", [skill])?;
            Ok(())
        })
    }
}
