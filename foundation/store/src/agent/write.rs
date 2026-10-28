//! Adding, superseding and retiring facts, and reading them back.

use super::*;

impl AgentStore {
    pub(super) fn insert_fact(
        &self,
        tx: &Transaction<'_>,
        f: &NewFact,
        vector: Option<&[f32]>,
    ) -> Result<i64, StoreError> {
        tx.execute(
            "INSERT INTO fact(text, kind, project, scope, cue, evidence, source, untrusted, confidence, model_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                f.text,
                f.kind.as_str(),
                f.project,
                f.scope.to_string(),
                f.cue,
                f.evidence,
                f.source,
                f.untrusted,
                f.confidence,
                vector.map(|_| self.model_id.as_str())
            ],
        )?;
        let id = tx.last_insert_rowid();
        if let Some(v) = vector.filter(|v| usable(v)) {
            tx.execute(
                "INSERT INTO fact_vec(rowid, embedding) VALUES (?1, ?2)",
                params![id, vec_blob(v)],
            )?;
            tx.execute("INSERT INTO ann_journal(op, fact_id) VALUES (1, ?1)", [id])?;
        }
        Ok(id)
    }

    pub(super) fn drop_vector(tx: &Transaction<'_>, id: i64) -> Result<(), StoreError> {
        if tx.execute("DELETE FROM fact_vec WHERE rowid = ?1", [id])? > 0 {
            tx.execute("INSERT INTO ann_journal(op, fact_id) VALUES (2, ?1)", [id])?;
        }
        Ok(())
    }

    /// ADD.
    pub fn add(&self, f: &NewFact, vector: Option<&[f32]>) -> Result<i64, StoreError> {
        if let Some(v) = vector {
            self.check_vec(v)?;
        }
        if f.text.trim().is_empty() {
            return Err(StoreError::Invalid("fact text is empty".into()));
        }
        let id = self.db.write(|tx| self.insert_fact(tx, f, vector))?;
        self.vectors.after_write();
        Ok(id)
    }

    /// UPDATE: a new fact supersedes `old` (history kept, decision 35). Returns the new id.
    pub fn supersede(
        &self,
        old: i64,
        f: &NewFact,
        vector: Option<&[f32]>,
    ) -> Result<i64, StoreError> {
        if let Some(v) = vector {
            self.check_vec(v)?;
        }
        self.db.write(|tx| {
            let live: Option<Option<i64>> =
                tx.query_row("SELECT superseded_by FROM fact WHERE id = ?1", [old], |r| r.get(0)).optional()?;
            match live {
                None => return Err(StoreError::NotFound(format!("fact {old}"))),
                Some(Some(_)) => return Err(StoreError::Conflict(format!("fact {old} is already superseded"))),
                Some(None) => {}
            }
            let id = self.insert_fact(tx, f, vector)?;
            tx.execute(
                "UPDATE fact SET superseded_by = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?1",
                params![old, id],
            )?;
            Self::drop_vector(tx, old)?;
            Ok(id)
        })
        .inspect(|_| self.vectors.after_write())
    }

    /// DELETE: retires a fact (self-superseded tombstone; the text stays for audit, it is never retrieved).
    pub fn retire(&self, id: i64) -> Result<(), StoreError> {
        self.db.write(|tx| {
            let n = tx.execute(
                "UPDATE fact SET superseded_by = id, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                 WHERE id = ?1 AND superseded_by IS NULL",
                [id],
            )?;
            if n == 0 {
                return Err(StoreError::NotFound(format!("live fact {id}")));
            }
            Self::drop_vector(tx, id)
        })?;
        self.vectors.after_write();
        Ok(())
    }

    pub fn get(&self, id: i64) -> Result<Fact, StoreError> {
        self.db
            .read(|c| {
                c.query_row(
                    &format!("SELECT {FACT_COLS} FROM fact WHERE id = ?1"),
                    [id],
                    map_fact,
                )
                .optional()
                .map_err(StoreError::from)
            })?
            .map(decode_fact)
            .transpose()?
            .ok_or_else(|| StoreError::NotFound(format!("fact {id}")))
    }

    pub(crate) fn facts_by_ids(
        c: &Connection,
        ids: &[i64],
    ) -> Result<HashMap<i64, Fact>, StoreError> {
        let mut out = HashMap::new();
        let mut st = c.prepare_cached(&format!("SELECT {FACT_COLS} FROM fact WHERE id = ?1"))?;
        for id in ids {
            if let Some(v) = st.query_row([id], map_fact).optional()? {
                out.insert(*id, decode_fact(v)?);
            }
        }
        Ok(out)
    }

    /// Live facts, newest first.
    pub fn live(&self, limit: usize) -> Result<Vec<Fact>, StoreError> {
        self.db.read(|c| {
            let mut st =
                c.prepare_cached(&format!("SELECT {FACT_COLS} FROM fact WHERE superseded_by IS NULL ORDER BY id DESC LIMIT ?1"))?;
            let rows = st.query_map([limit as i64], map_fact)?;
            rows.map(|r| decode_fact(r?)).collect()
        })
    }

    /// Live, trusted lessons whose scope holds in `ctx`: the lesson has a scope, and each of its keys has the same
    /// value in `ctx` (decision 35: the rules of that context). The most proven first: held in more contexts, then
    /// more confident, then newer.
    pub fn rules_in(&self, ctx: &serde_json::Value, limit: usize) -> Result<Vec<Fact>, StoreError> {
        let ctx = crate::graph::normalize_scope(ctx);
        let Some(c) = ctx.as_object().filter(|c| !c.is_empty()) else {
            return Ok(vec![]);
        };
        let mut out: Vec<Fact> = self.db.read(|db| {
            let mut st = db.prepare_cached(&format!(
                "SELECT {FACT_COLS} FROM fact WHERE superseded_by IS NULL AND kind = 'lesson' AND untrusted = 0"
            ))?;
            let rows = st.query_map([], map_fact)?;
            rows.map(|r| decode_fact(r?)).collect::<Result<Vec<_>, StoreError>>()
        })?;
        out.retain(|f| {
            f.weight >= HIDE_BELOW_WEIGHT
                && crate::graph::normalize_scope(&f.scope)
                    .as_object()
                    .is_some_and(|o| !o.is_empty() && o.iter().all(|(k, v)| c.get(k) == Some(v)))
        });
        out.sort_by(|a, b| {
            b.validated
                .len()
                .cmp(&a.validated.len())
                .then(b.confidence.total_cmp(&a.confidence))
                .then(b.id.cmp(&a.id))
        });
        out.truncate(limit);
        Ok(out)
    }
}
