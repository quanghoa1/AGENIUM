//! Entities: found, added or updated, matched in text, merged, re-embedded.

use super::*;

impl AgentStore {
    pub fn entity(&self, id: i64) -> Result<Entity, StoreError> {
        self.db.read(|c| {
            c.query_row(
                &format!("SELECT {ENTITY_COLS} FROM entity WHERE id = ?1"),
                [id],
                map_entity,
            )
            .optional()?
            .map(decode_entity)
            .transpose()?
            .ok_or_else(|| StoreError::NotFound(format!("entity {id}")))
        })
    }

    /// Follows `merged_into` to the surviving entity.
    pub fn live_entity(&self, id: i64) -> Result<Entity, StoreError> {
        let mut e = self.entity(id)?;
        for _ in 0..32 {
            match e.merged_into {
                Some(n) => e = self.entity(n)?,
                None => return Ok(e),
            }
        }
        Err(StoreError::Corrupt(format!(
            "merge chain of entity {id} does not end"
        )))
    }

    /// Inserts an entity or, when one with the same type + canonical name + identity scope exists, records the new
    /// mention (aliases, `mentions`, `last_seen_at`). Returns `(id, created)`.
    pub fn upsert_entity(
        &self,
        e: &NewEntity,
        vector: Option<&[f32]>,
    ) -> Result<(i64, bool), StoreError> {
        if let Some(v) = vector {
            self.check_vec(v)?;
        }
        let key = canonical(&e.name);
        if key.is_empty() || e.kind.trim().is_empty() {
            return Err(StoreError::Invalid("entity needs a type and a name".into()));
        }
        let kind = canonical(&e.kind).replace(' ', "_");
        let scope = normalize_scope(&e.scope);
        let sk = scope_key(&scope);
        self.db.write(|tx| {
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM entity WHERE type = ?1 AND canonical_key = ?2 AND scope_key = ?3 AND merged_into IS NULL",
                    params![kind, key, sk],
                    |r| r.get(0),
                )
                .optional()?;
            let (id, created) = match existing {
                Some(id) => {
                    tx.execute(
                        &format!(
                            "UPDATE entity SET mentions = mentions + 1, last_seen_at = {n}, updated_at = {n},
                             description = COALESCE(description, ?2) WHERE id = ?1",
                            n = now()
                        ),
                        params![id, e.description],
                    )?;
                    (id, false)
                }
                None => {
                    let evidence: Vec<&str> = e.evidence.iter().map(String::as_str).collect();
                    tx.execute(
                        &format!(
                            "INSERT INTO entity(type, name, canonical_key, scope, scope_key, description, confidence, evidence, last_seen_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, {})",
                            now()
                        ),
                        params![
                            kind,
                            e.name.trim(),
                            key,
                            scope.to_string(),
                            sk,
                            e.description,
                            e.confidence.clamp(0.0, 1.0),
                            serde_json::to_string(&evidence).unwrap_or_else(|_| "[]".into())
                        ],
                    )?;
                    let id = tx.last_insert_rowid();
                    if let Some(v) = vector.filter(|v| usable(v)) {
                        tx.execute(
                            "INSERT INTO entity_vec(rowid, embedding) VALUES (?1, ?2)",
                            params![id, vec_blob(v)],
                        )?;
                    }
                    (id, true)
                }
            };
            add_alias(tx, id, &e.name, "extract")?;
            for a in &e.aliases {
                add_alias(tx, id, a, "extract")?;
            }
            refresh_fts(tx, id)?;
            Ok((id, created))
        })
    }

    /// Entity linking candidates for a mention of `kind` (ADR-51 §4): exact key, alias, FTS on name/aliases,
    /// vector KNN on name + description. Only live, non-merged entities of the same type.
    pub fn entity_candidates(
        &self,
        kind: &str,
        name: &str,
        vector: Option<&[f32]>,
        k: usize,
    ) -> Result<Vec<Candidate>, StoreError> {
        let kind = canonical(kind).replace(' ', "_");
        let key = canonical(name);
        let mut found: BTreeMap<i64, (&'static str, f64)> = BTreeMap::new();
        self.db.read(|c| -> Result<(), StoreError> {
            let mut st = c.prepare_cached(
                "SELECT id FROM entity WHERE type = ?1 AND canonical_key = ?2 AND merged_into IS NULL AND valid_to IS NULL",
            )?;
            for id in st.query_map(params![kind, key], |r| r.get::<_, i64>(0))? {
                found.insert(id?, ("exact", 1.0));
            }
            let mut st = c.prepare_cached(
                "SELECT e.id FROM entity_alias a JOIN entity e ON e.id = a.entity_id
                 WHERE a.alias_norm = ?1 AND e.type = ?2 AND e.merged_into IS NULL AND e.valid_to IS NULL",
            )?;
            for id in st.query_map(params![key, kind], |r| r.get::<_, i64>(0))? {
                found.entry(id?).or_insert(("alias", 0.99));
            }
            if let Some(q) = crate::agent::fts_query(name) {
                let mut st = c.prepare_cached(
                    "SELECT e.id FROM entity_fts JOIN entity e ON e.id = entity_fts.rowid
                     WHERE entity_fts MATCH ?1 AND e.type = ?2 AND e.merged_into IS NULL AND e.valid_to IS NULL
                     ORDER BY bm25(entity_fts) LIMIT ?3",
                )?;
                for id in st.query_map(params![q, kind, k as i64], |r| r.get::<_, i64>(0))? {
                    found.entry(id?).or_insert(("fts", 0.0));
                }
            }
            if let Some(v) = vector.filter(|v| usable(v)) {
                let mut st = c.prepare_cached(
                    "SELECT rowid, distance FROM entity_vec WHERE embedding MATCH ?1 AND k = ?2 ORDER BY distance",
                )?;
                let rows: Vec<(i64, Option<f64>)> = st
                    .query_map(params![vec_blob(v), (k * 4) as i64], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<_, _>>()?;
                for (id, d) in rows.into_iter().filter_map(|(id, d)| Some((id, d?))) {
                    let sim = 1.0 - d;
                    let e = found.entry(id).or_insert(("vector", sim));
                    if e.1 < sim {
                        e.1 = sim;
                    }
                }
            }
            Ok(())
        })?;
        let mut out = Vec::new();
        for (id, (via, sim)) in found {
            let e = self.entity(id)?;
            if e.kind == kind && e.merged_into.is_none() {
                out.push(Candidate {
                    entity: e,
                    via,
                    similarity: sim,
                });
            }
        }
        out.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
        out.truncate(k);
        Ok(out)
    }

    /// Entities whose name or alias occurs in `text` as whole words (query-side linking; longest names first).
    pub fn entities_in_text(&self, text: &str, limit: usize) -> Result<Vec<Entity>, StoreError> {
        let norm = format!(" {} ", canonical(text));
        let words: Vec<&str> = norm.split_whitespace().collect();
        if words.is_empty() {
            return Ok(vec![]);
        }
        // Candidate n-grams up to 5 words.
        let mut grams = Vec::new();
        for n in (1..=5).rev() {
            for w in words.windows(n) {
                grams.push(w.join(" "));
            }
        }
        grams.sort();
        grams.dedup();
        let ids: Vec<i64> = self.db.read(|c| -> Result<Vec<i64>, StoreError> {
            let mut st = c.prepare_cached(
                "SELECT DISTINCT e.id, length(a.alias_norm) FROM entity_alias a JOIN entity e ON e.id = a.entity_id
                 WHERE a.alias_norm = ?1 AND e.merged_into IS NULL AND e.valid_to IS NULL",
            )?;
            let mut hits: Vec<(i64, i64)> = Vec::new();
            for g in &grams {
                if g.chars().count() < 2 {
                    continue;
                }
                for r in st.query_map([g], |r| Ok((r.get(0)?, r.get(1)?)))? {
                    hits.push(r?);
                }
            }
            hits.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let mut seen = std::collections::HashSet::new();
            Ok(hits.into_iter().filter(|(id, _)| seen.insert(*id)).map(|(id, _)| id).take(limit).collect())
        })?;
        ids.into_iter().map(|id| self.entity(id)).collect()
    }

    /// Merges `from` into `into` (entity resolution): edges and fact links are rewired (an edge that becomes a
    /// duplicate or a self-loop is ended), aliases move, `from` becomes a tombstone. Logged for undo.
    pub fn merge_entities(
        &self,
        from: i64,
        into: i64,
        similarity: Option<f64>,
        decided_by: &str,
    ) -> Result<(), StoreError> {
        if from == into {
            return Err(StoreError::Invalid(
                "cannot merge an entity into itself".into(),
            ));
        }
        let (a, b) = (self.entity(from)?, self.entity(into)?);
        if a.kind != b.kind {
            return Err(StoreError::Invalid(format!(
                "will not merge a {} into a {}",
                a.kind, b.kind
            )));
        }
        if a.merged_into.is_some() || b.merged_into.is_some() {
            return Err(StoreError::Invalid("entity already merged".into()));
        }
        if matches!(scope_match(&a.scope, &b.scope), ScopeMatch::Conflict) {
            return Err(StoreError::Invalid(
                "identity scopes conflict; not the same entity".into(),
            ));
        }
        self.db.write(|tx| {
            let mut moved_edges = Vec::new();
            let mut ended = Vec::new();
            let mut st = tx.prepare("SELECT id, src, dst, type, scope_key FROM edge WHERE (src = ?1 OR dst = ?1) AND valid_to IS NULL")?;
            let edges: Vec<(i64, i64, i64, String, String)> = st
                .query_map([from], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
                .collect::<Result<_, _>>()?;
            drop(st);
            for (id, s, d, t, sk) in edges {
                let (ns, nd) = (if s == from { into } else { s }, if d == from { into } else { d });
                let dup: Option<i64> = tx
                    .query_row(
                        "SELECT id FROM edge WHERE src = ?1 AND dst = ?2 AND type = ?3 AND scope_key = ?4 AND valid_to IS NULL AND id <> ?5",
                        params![ns, nd, t, sk, id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if ns == nd || dup.is_some() {
                    tx.execute(
                        &format!("UPDATE edge SET valid_to = {n}, superseded_by = ?2, updated_at = {n} WHERE id = ?1", n = now()),
                        params![id, dup],
                    )?;
                    if let Some(k) = dup {
                        tx.execute("UPDATE edge SET weight = weight + 1 WHERE id = ?1", [k])?;
                    }
                    bump_degree(tx, &[s, d], -1)?;
                    ended.push(id);
                } else {
                    tx.execute("UPDATE edge SET src = ?2, dst = ?3 WHERE id = ?1", params![id, ns, nd])?;
                    bump_degree(tx, &[from], -1)?;
                    bump_degree(tx, &[into], 1)?;
                    moved_edges.push(id);
                }
            }
            let links = tx.execute(
                "INSERT OR IGNORE INTO fact_entity(fact_id, entity_id, role, confidence)
                 SELECT fact_id, ?2, role, confidence FROM fact_entity WHERE entity_id = ?1",
                params![from, into],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO entity_alias(entity_id, alias_norm, alias, source)
                 SELECT ?2, alias_norm, alias, 'merge' FROM entity_alias WHERE entity_id = ?1",
                params![from, into],
            )?;
            tx.execute(
                &format!(
                    "UPDATE entity SET mentions = mentions + (SELECT mentions FROM entity WHERE id = ?1), updated_at = {} WHERE id = ?2",
                    now()
                ),
                params![from, into],
            )?;
            tx.execute(
                &format!("UPDATE entity SET merged_into = ?2, updated_at = {} WHERE id = ?1", now()),
                params![from, into],
            )?;
            tx.execute("DELETE FROM entity_vec WHERE rowid = ?1", [from])?;
            tx.execute("DELETE FROM entity_fts WHERE rowid = ?1", [from])?;
            refresh_fts(tx, into)?;
            let moved = serde_json::json!({ "edges": moved_edges, "ended_edges": ended, "fact_links": links });
            tx.execute(
                "INSERT INTO entity_merge_log(from_id, into_id, similarity, decided_by, moved) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![from, into, similarity, decided_by, moved.to_string()],
            )?;
            Ok(())
        })
    }

    /// Rebuilds `entity_fts` and `entity_vec` (after an import or an embedder change). Vectors are made from the name
    /// alone, as `agenium_memory::resolve::entity_text` makes them (ADR-94).
    pub fn reindex_entities(
        &self,
        embed: impl Fn(&[String]) -> Result<Vec<Vec<f32>>, String>,
    ) -> Result<u64, StoreError> {
        let rows: Vec<(i64, String)> = self.db.read(|c| -> Result<_, StoreError> {
            let mut st =
                c.prepare("SELECT id, name || '. ' FROM entity WHERE merged_into IS NULL")?;
            let v = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })?;
        let texts: Vec<String> = rows.iter().map(|r| r.1.clone()).collect();
        let vecs = if texts.is_empty() {
            vec![]
        } else {
            embed(&texts).map_err(StoreError::Invalid)?
        };
        if vecs.len() != rows.len() {
            return Err(StoreError::Invalid(
                "embedder returned a different number of vectors".into(),
            ));
        }
        for v in &vecs {
            self.check_vec(v)?;
        }
        self.db.write(|tx| {
            tx.execute("DELETE FROM entity_vec", [])?;
            for ((id, _), v) in rows.iter().zip(&vecs) {
                if usable(v) {
                    tx.execute(
                        "INSERT INTO entity_vec(rowid, embedding) VALUES (?1, ?2)",
                        params![id, vec_blob(v)],
                    )?;
                }
                refresh_fts(tx, *id)?;
            }
            Ok(rows.len() as u64)
        })
    }
}
