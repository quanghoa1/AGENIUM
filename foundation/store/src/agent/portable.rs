//! The conversation store, export and import, and the store's key-value meta.

use super::*;

impl AgentStore {
    pub fn append_message(
        &self,
        session: &str,
        message: &serde_json::Value,
    ) -> Result<u32, StoreError> {
        self.db.write(|tx| {
            let n: i64 = tx.query_row(
                "SELECT COALESCE(MAX(n), 0) + 1 FROM conversation WHERE session = ?1",
                [session],
                |r| r.get(0),
            )?;
            tx.execute(
                "INSERT INTO conversation(session, n, message) VALUES (?1, ?2, ?3)",
                params![session, n, message.to_string()],
            )?;
            Ok(n as u32)
        })
    }

    /// Replaces a session's whole conversation (ADR-155: a run's conversation folded to fit its model's window).
    pub fn replace_messages(
        &self,
        session: &str,
        messages: &[serde_json::Value],
    ) -> Result<(), StoreError> {
        self.db.write(|tx| {
            tx.execute("DELETE FROM conversation WHERE session = ?1", [session])?;
            for (i, m) in messages.iter().enumerate() {
                tx.execute(
                    "INSERT INTO conversation(session, n, message) VALUES (?1, ?2, ?3)",
                    params![session, (i + 1) as i64, m.to_string()],
                )?;
            }
            Ok(())
        })
    }

    pub fn load_messages(&self, session: &str) -> Result<Vec<serde_json::Value>, StoreError> {
        self.db.read(|c| {
            let mut st =
                c.prepare_cached("SELECT message FROM conversation WHERE session = ?1 ORDER BY n")?;
            let rows = st.query_map([session], |r| r.get::<_, String>(0))?;
            rows.map(|s| crate::core::from_json(&s?)).collect()
        })
    }

    // ------------------------------------------------------------ export / import (M15-5)

    /// Writes the agent's memory as JSONL lines: per table `{"kind":"table","table":..,"cols":[..]}` followed by
    /// `{"kind":"row","v":[..]}` lines (rowid order, values typed as in incremental backups). Vectors are not
    /// exported: text is the source of truth and the importer re-embeds.
    pub fn export_jsonl(&self, w: &mut dyn std::io::Write) -> Result<u64, StoreError> {
        self.db.read(|c| -> Result<u64, StoreError> {
            c.execute_batch("BEGIN")?;
            let r = (|| -> Result<u64, StoreError> {
                let mut n = 0u64;
                for t in EXPORT_TABLES {
                    let order = backup::row_order(c, t)?;
                    let mut st = c.prepare(&format!("SELECT * FROM {t} ORDER BY {order}"))?;
                    let cols: Vec<String> = st.column_names().map(|s| s.to_string()).collect();
                    let header = serde_json::json!({ "kind": "table", "table": t, "cols": cols });
                    writeln!(w, "{header}").map_err(|e| StoreError::Invalid(e.to_string()))?;
                    let k = st.column_count();
                    let mut rows = st.query([])?;
                    while let Some(r) = rows.next()? {
                        let vals: Vec<serde_json::Value> = (0..k)
                            .map(|i| r.get_ref(i).map(backup::value_to_json))
                            .collect::<Result<_, _>>()?;
                        let line = serde_json::json!({ "kind": "row", "v": vals });
                        writeln!(w, "{line}").map_err(|e| StoreError::Invalid(e.to_string()))?;
                        n += 1;
                    }
                }
                Ok(n)
            })();
            c.execute_batch("COMMIT")?;
            r
        })
    }

    /// Loads lines written by `export_jsonl` into this (empty) database in one transaction; fact ids are kept so
    /// `superseded_by` and usage rows stay linked. Returns the number of rows. Call `reindex` afterwards.
    pub fn import_jsonl(&self, lines: &[serde_json::Value]) -> Result<u64, StoreError> {
        let existing: i64 = self.db.read(|c| -> Result<i64, StoreError> {
            Ok(c.query_row(
                "SELECT (SELECT count(*) FROM fact) + (SELECT count(*) FROM conversation)",
                [],
                |r| r.get(0),
            )?)
        })?;
        if existing > 0 {
            return Err(StoreError::Invalid(
                "the agent database is not empty; import needs a new agent".into(),
            ));
        }
        self.db.write(|tx| {
            tx.execute_batch("PRAGMA defer_foreign_keys = ON")?;
            let mut sql: Option<String> = None;
            let mut n = 0u64;
            for l in lines {
                match l["kind"].as_str() {
                    Some("table") => {
                        let t = l["table"].as_str().unwrap_or_default();
                        if !EXPORT_TABLES.contains(&t) {
                            return Err(StoreError::Invalid(format!("unexpected table {t:?}")));
                        }
                        let cols: Vec<String> = l["cols"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|c| c.as_str())
                            .map(|c| {
                                if c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
                                    Ok(c.to_owned())
                                } else {
                                    Err(StoreError::Invalid(format!("bad column {c:?}")))
                                }
                            })
                            .collect::<Result<_, _>>()?;
                        sql = Some(format!(
                            "INSERT INTO {t} ({}) VALUES ({})",
                            cols.join(", "),
                            vec!["?"; cols.len()].join(", ")
                        ));
                    }
                    Some("row") => {
                        let Some(q) = &sql else {
                            return Err(StoreError::Invalid("row before its table header".into()));
                        };
                        let vals: Vec<rusqlite::types::Value> = l["v"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(backup::json_to_value)
                            .collect::<Result<_, _>>()?;
                        tx.execute(q, rusqlite::params_from_iter(vals))?;
                        n += 1;
                    }
                    _ => {}
                }
            }
            Ok(n)
        })
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>, StoreError> {
        self.db.read(|c| {
            Ok(
                c.query_row("SELECT value FROM agent_meta WHERE key = ?1", [key], |r| {
                    r.get(0)
                })
                .optional()?,
            )
        })
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO agent_meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
            Ok(())
        })
    }
}
