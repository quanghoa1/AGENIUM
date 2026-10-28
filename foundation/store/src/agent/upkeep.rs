//! Usage and upkeep: what was shown and used, validation, decay, consolidation, and re-embedding.

use super::*;

impl AgentStore {
    /// Logs facts injected into a work order (M1-11) and touches `used_at`. A fact logged twice for one run keeps its
    /// first row; any other failure is an error (`INSERT OR IGNORE` also swallowed CHECK failures: rule-injected
    /// lessons were dropped without a word, see agent_004.sql).
    pub fn record_usage(
        &self,
        run: &str,
        ticket: Option<&str>,
        used: &[(i64, u32, Via)],
    ) -> Result<(), StoreError> {
        let rows: Vec<(i64, u32, Via, bool)> =
            used.iter().map(|(f, r, v)| (*f, *r, *v, false)).collect();
        self.record_shown(run, ticket, &rows)
    }

    /// [`Self::record_usage`] with, per fact, whether it was shown as a rule of the work's context (a lesson whose
    /// scope matched exactly, ADR-118).
    pub fn record_shown(
        &self,
        run: &str,
        ticket: Option<&str>,
        used: &[(i64, u32, Via, bool)],
    ) -> Result<(), StoreError> {
        self.db.write(|tx| {
            for (fact, rank, via, as_rule) in used {
                tx.execute(
                    "INSERT INTO fact_usage(fact, ticket, run, rank, via, as_rule) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(run, fact) DO NOTHING",
                    params![fact, ticket, run, *rank as i64, via.as_str(), *as_rule as i64],
                )?;
                tx.execute("UPDATE fact SET used_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?1", [fact])?;
            }
            Ok(())
        })
    }

    /// Records that a fact held in `context` (`project:<id>` / `ticket:<key>`); returns in how many distinct contexts
    /// it has held (decision 35: two or more make a lesson a skill-update candidate).
    pub fn validate_in(&self, fact: i64, context: &str) -> Result<usize, StoreError> {
        self.db.write(|tx| {
            let raw: String =
                tx.query_row("SELECT validated FROM fact WHERE id = ?1", [fact], |r| {
                    r.get(0)
                })?;
            let mut v: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
            if !v.iter().any(|c| c == context) {
                v.push(context.to_owned());
                tx.execute(
                    "UPDATE fact SET validated = ?2 WHERE id = ?1",
                    params![
                        fact,
                        serde_json::to_string(&v).unwrap_or_else(|_| "[]".into())
                    ],
                )?;
            }
            Ok(v.len())
        })
    }

    /// Forgetting (decision 35): the weight of every live fact halves for each `half_life_days` it went unused (since
    /// it was last used, else since it was made); below [`HIDE_BELOW_WEIGHT`] it is no longer retrieved but kept.
    /// Returns how many weights changed.
    pub fn decay(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        half_life_days: f64,
    ) -> Result<u64, StoreError> {
        self.db.write(|tx| {
            let rows: Vec<(i64, String, Option<String>, f64)> = {
                let mut st = tx.prepare(
                    "SELECT id, created_at, used_at, weight FROM fact WHERE superseded_by IS NULL",
                )?;
                let rows = st
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            let mut n = 0;
            for (id, created, used, old) in rows {
                let since = used.unwrap_or(created);
                let Ok(at) = chrono::DateTime::parse_from_rfc3339(&since) else {
                    continue;
                };
                let days =
                    (now - at.with_timezone(&chrono::Utc)).num_seconds().max(0) as f64 / 86_400.0;
                let w = 0.5f64.powf(days / half_life_days.max(1.0));
                if (w - old).abs() > 1e-6 {
                    tx.execute("UPDATE fact SET weight = ?2 WHERE id = ?1", params![id, w])?;
                    n += 1;
                }
            }
            Ok(n)
        })
    }

    /// Consolidation (decision 35): live facts that say the same thing (same kind, scope, project, trust and text up
    /// to case and spacing) become one: the most confident (then the newest) stays, the others are superseded by it
    /// and keep their history; the contexts they were validated in move to the one that stays. Trust and project are
    /// part of the key: an untrusted copy (made from tool output or another agent's words) never replaces a trusted
    /// fact, and a fact learned in one project is not folded into another's. Returns how many were folded.
    pub fn consolidate(&self) -> Result<u64, StoreError> {
        // (kind, scope, project, untrusted, normalised text) and (id, text, kind, scope, project, untrusted,
        // confidence, validated).
        type Key = (String, String, Option<String>, bool, String);
        type Row = (
            i64,
            String,
            String,
            String,
            Option<String>,
            bool,
            f64,
            String,
        );
        self.db.write(|tx| {
            let rows: Vec<Row> = {
                let mut st = tx.prepare(
                    "SELECT id, text, kind, scope, project, untrusted, confidence, validated FROM fact
                     WHERE superseded_by IS NULL AND id IS NOT NULL",
                )?;
                let rows = st
                    .query_map([], |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                            r.get(7)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            let norm = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
            let mut groups: std::collections::BTreeMap<Key, Vec<(i64, f64, String)>> =
                std::collections::BTreeMap::new();
            for (id, text, kind, scope, project, untrusted, conf, validated) in rows {
                groups
                    .entry((kind, scope, project, untrusted, norm(&text)))
                    .or_default()
                    .push((id, conf, validated));
            }
            let mut n = 0;
            for (_, mut ids) in groups.into_iter().filter(|(_, v)| v.len() > 1) {
                ids.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.0.cmp(&a.0)));
                let keep = ids[0].0;
                let contexts = |raw: &str| serde_json::from_str::<Vec<String>>(raw).unwrap_or_default();
                let mut validated = contexts(&ids[0].2);
                for (id, _, other) in &ids[1..] {
                    for c in contexts(other) {
                        if !validated.contains(&c) {
                            validated.push(c);
                        }
                    }
                    tx.execute(
                        "UPDATE fact SET superseded_by = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?1",
                        params![id, keep],
                    )?;
                    Self::drop_vector(tx, *id)?;
                    n += 1;
                }
                if validated != contexts(&ids[0].2) {
                    tx.execute(
                        "UPDATE fact SET validated = ?2 WHERE id = ?1",
                        params![keep, serde_json::to_string(&validated)?],
                    )?;
                }
            }
            Ok(n)
        })
    }

    /// Forgetting and consolidation now (decision 35): `(weights changed, facts folded)`.
    pub fn tidy(&self) -> Result<(u64, u64), StoreError> {
        Ok((
            self.decay(chrono::Utc::now(), DECAY_HALF_LIFE_DAYS)?,
            self.consolidate()?,
        ))
    }

    /// Maintenance tick for this agent's database: passive checkpoint, and the WAL gate past the limit. Returns the WAL
    /// size afterwards. (Writes also run the WAL gate themselves; the tick catches databases that only read.)
    /// Also saves unsaved sidecar changes.
    pub fn maintain(&self) -> Result<u64, StoreError> {
        self.vectors.maintain()?;
        self.db.maintain()
    }

    /// Saves the ANN sidecar now (it is also saved every `save_every` changes, by `maintain` and on close).
    pub fn flush(&self) -> Result<(), StoreError> {
        self.vectors.save()
    }

    /// Lessons that were in play in `run` (ADR-118): shown as a rule of its context and found by the work's own words
    /// (a text or graph channel), not only put in front of every run of the project.
    pub fn lessons_in_play(
        &self,
        run: &str,
    ) -> Result<std::collections::BTreeSet<i64>, StoreError> {
        self.db.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT fact FROM fact_usage WHERE run = ?1 AND as_rule = 1 AND via NOT IN ('rule', 'recent')",
            )?;
            let rows = st.query_map([run], |r| r.get::<_, i64>(0))?;
            rows.collect::<Result<_, _>>().map_err(Into::into)
        })
    }

    pub fn usage_for_run(&self, run: &str) -> Result<Vec<(i64, u32, String)>, StoreError> {
        self.db.read(|c| {
            let mut st = c.prepare_cached(
                "SELECT fact, rank, via FROM fact_usage WHERE run = ?1 ORDER BY rank",
            )?;
            let rows = st.query_map([run], |r| {
                Ok((r.get(0)?, r.get::<_, i64>(1)? as u32, r.get(2)?))
            })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
        })
    }

    /// Live facts whose vector is missing or comes from another embedder than `model_id`.
    pub fn stale_vectors(&self, model_id: &str) -> Result<u64, StoreError> {
        self.db.read(|c| -> Result<u64, StoreError> {
            Ok(c.query_row(
                "SELECT count(*) FROM fact WHERE superseded_by IS NULL AND (model_id IS NULL OR model_id <> ?1)",
                [model_id],
                |r| r.get::<_, i64>(0),
            )? as u64)
        })
    }

    /// Replaces the vectors of every live fact (embedder change with the same dimension).
    pub fn reindex(
        &self,
        embed: impl Fn(&[String]) -> Result<Vec<Vec<f32>>, String>,
    ) -> Result<u64, StoreError> {
        let facts = self.live(usize::MAX >> 1)?;
        let texts: Vec<String> = facts.iter().map(|f| f.text.clone()).collect();
        let vecs = embed(&texts).map_err(StoreError::Invalid)?;
        if vecs.len() != facts.len() {
            return Err(StoreError::Invalid(
                "embedder returned a different number of vectors".into(),
            ));
        }
        for v in &vecs {
            self.check_vec(v)?;
        }
        self.db
            .write(|tx| {
                for (f, v) in facts.iter().zip(&vecs) {
                    let had = tx.execute("DELETE FROM fact_vec WHERE rowid = ?1", [f.id])? > 0;
                    tx.execute(
                        "UPDATE fact SET model_id = ?2 WHERE id = ?1",
                        params![f.id, self.model_id],
                    )?;
                    if !usable(v) {
                        if had {
                            tx.execute(
                                "INSERT INTO ann_journal(op, fact_id) VALUES (2, ?1)",
                                [f.id],
                            )?;
                        }
                        continue;
                    }
                    tx.execute(
                        "INSERT INTO fact_vec(rowid, embedding) VALUES (?1, ?2)",
                        params![f.id, vec_blob(v)],
                    )?;
                    tx.execute(
                        "INSERT INTO ann_journal(op, fact_id) VALUES (1, ?1)",
                        [f.id],
                    )?;
                }
                Ok(facts.len() as u64)
            })
            .inspect(|_| self.vectors.after_write())
    }

    // ------------------------------------------------------------ conversation (resumable run context)
}
