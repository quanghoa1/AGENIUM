//! Edges between entities and the links from facts to entities.

use super::*;

impl AgentStore {
    /// Adds a live edge or strengthens the identical live one (`weight`, `confidence` max). Returns its id.
    #[allow(clippy::too_many_arguments)]
    pub fn add_edge(
        &self,
        src: i64,
        dst: i64,
        kind: &str,
        scope: &Value,
        confidence: f64,
        evidence_fact: Option<i64>,
        source: &str,
    ) -> Result<i64, StoreError> {
        if src == dst {
            return Err(StoreError::Invalid(
                "an edge needs two different entities".into(),
            ));
        }
        let scope = normalize_scope(scope);
        let sk = scope_key(&scope);
        self.db.write(|tx| {
            let known: Option<i64> = tx
                .query_row("SELECT 1 FROM edge_type WHERE name = ?1", [kind], |r| r.get(0))
                .optional()?;
            if known.is_none() {
                return Err(StoreError::Invalid(format!("unknown edge type {kind:?}")));
            }
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM edge WHERE src = ?1 AND dst = ?2 AND type = ?3 AND scope_key = ?4 AND valid_to IS NULL",
                    params![src, dst, kind, sk],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = existing {
                tx.execute(
                    &format!(
                        "UPDATE edge SET weight = weight + 1, confidence = MAX(confidence, ?2),
                         evidence_fact = COALESCE(evidence_fact, ?3), updated_at = {} WHERE id = ?1",
                        now()
                    ),
                    params![id, confidence.clamp(0.0, 1.0), evidence_fact],
                )?;
                return Ok(id);
            }
            tx.execute(
                "INSERT INTO edge(src, dst, type, scope, scope_key, confidence, evidence_fact, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![src, dst, kind, scope.to_string(), sk, confidence.clamp(0.0, 1.0), evidence_fact, source],
            )?;
            let id = tx.last_insert_rowid();
            bump_degree(tx, &[src, dst], 1)?;
            Ok(id)
        })
    }

    /// Ends an edge (contradicted or replaced); it stays for audit.
    pub fn invalidate_edge(&self, id: i64, superseded_by: Option<i64>) -> Result<(), StoreError> {
        self.db.write(|tx| {
            let ends: Option<(i64, i64)> = tx
                .query_row("SELECT src, dst FROM edge WHERE id = ?1 AND valid_to IS NULL", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?;
            let Some((s, d)) = ends else {
                return Ok(());
            };
            tx.execute(
                &format!("UPDATE edge SET valid_to = {n}, superseded_by = ?2, updated_at = {n} WHERE id = ?1", n = now()),
                params![id, superseded_by],
            )?;
            bump_degree(tx, &[s, d], -1)?;
            Ok(())
        })
    }

    pub fn edges_of(&self, entity: i64) -> Result<Vec<Edge>, StoreError> {
        self.db.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT id, src, dst, type, scope, weight, confidence, evidence_fact, valid_to FROM edge
                 WHERE src = ?1 OR dst = ?1 ORDER BY id",
            )?;
            let rows = st.query_map([entity], |r| {
                Ok((
                    Edge {
                        id: r.get(0)?,
                        src: r.get(1)?,
                        dst: r.get(2)?,
                        kind: r.get(3)?,
                        scope: Value::Null,
                        weight: r.get(5)?,
                        confidence: r.get(6)?,
                        evidence_fact: r.get(7)?,
                        valid_to: r.get(8)?,
                    },
                    r.get::<_, String>(4)?,
                ))
            })?;
            rows.map(|r| {
                let (mut e, s) = r?;
                e.scope = crate::core::from_json(&s)?;
                Ok(e)
            })
            .collect()
        })
    }

    pub fn link_fact(&self, fact: i64, entity: i64, role: &str) -> Result<(), StoreError> {
        let role = match role {
            "subject" | "object" | "scope" => role,
            _ => "mention",
        };
        self.db.write(|tx| {
            tx.execute(
                "INSERT OR IGNORE INTO fact_entity(fact_id, entity_id, role) VALUES (?1, ?2, ?3)",
                params![fact, entity, role],
            )?;
            Ok(())
        })
    }

    /// Entities a fact is about.
    pub fn fact_entities(&self, fact: i64) -> Result<Vec<i64>, StoreError> {
        self.db.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT DISTINCT entity_id FROM fact_entity WHERE fact_id = ?1 ORDER BY entity_id",
            )?;
            let rows = st.query_map([fact], |r| r.get(0))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }
}
