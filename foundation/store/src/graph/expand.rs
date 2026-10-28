//! From entities to facts: the expansion over the graph and the confidence signals of use.

use super::*;

impl AgentStore {
    /// Bounded two-arm expansion from `seeds` (ADR-51 §6a): returns reached nodes (not the seeds) with the best
    /// hop-decayed score. Edges whose scope conflicts with `ctx` are not crossed; hubs are not expanded; the query
    /// is interrupted after `x.budget` (whatever the vector/keyword channels found is still used).
    pub fn expand(
        &self,
        seeds: &[i64],
        ctx: &Value,
        x: &Expansion,
    ) -> Result<ExpandResult, StoreError> {
        if seeds.is_empty() || x.max_depth == 0 {
            return Ok(ExpandResult::default());
        }
        let ctx = normalize_scope(ctx).to_string();
        let seeds_json = serde_json::to_string(seeds).unwrap_or_else(|_| "[]".into());
        let depth = x.max_depth.min(3) as i64;
        let sql = "
WITH RECURSIVE walk(node, depth, score, path) AS (
  SELECT value, 0, 1.0, ',' || value || ',' FROM json_each(?1)
  UNION ALL
  SELECT e.dst, w.depth + 1, w.score * t.hop_weight * e.confidence, w.path || e.dst || ','
  FROM walk w
  JOIN entity n ON n.id = w.node AND n.degree <= ?3
  JOIN edge e INDEXED BY edge_out ON e.src = w.node AND e.valid_to IS NULL
  JOIN edge_type t ON t.name = e.type
  WHERE w.depth < ?2 AND t.traverse = 1
    AND instr(w.path, ',' || e.dst || ',') = 0
    AND NOT EXISTS (SELECT 1 FROM json_each(e.scope) s
                    WHERE json_extract(?4, '$.' || s.key) IS NOT NULL AND json_extract(?4, '$.' || s.key) <> s.value)
  UNION ALL
  SELECT e.src, w.depth + 1, w.score * t.hop_weight * e.confidence, w.path || e.src || ','
  FROM walk w
  JOIN entity n ON n.id = w.node AND n.degree <= ?3
  JOIN edge e INDEXED BY edge_in ON e.dst = w.node AND e.valid_to IS NULL
  JOIN edge_type t ON t.name = e.type
  WHERE w.depth < ?2 AND t.traverse = 1
    AND instr(w.path, ',' || e.src || ',') = 0
    AND NOT EXISTS (SELECT 1 FROM json_each(e.scope) s
                    WHERE json_extract(?4, '$.' || s.key) IS NOT NULL AND json_extract(?4, '$.' || s.key) <> s.value)
)
SELECT node, MAX(score) AS score, MIN(depth) AS hops FROM walk
WHERE depth > 0 AND node NOT IN (SELECT value FROM json_each(?1))
GROUP BY node ORDER BY score DESC, node LIMIT ?5";
        self.db.read(|c| -> Result<ExpandResult, StoreError> {
            let start = Instant::now();
            let budget = x.budget;
            c.progress_handler(1000, Some(move || start.elapsed() > budget))?;
            let r = (|| -> Result<Vec<Reached>, rusqlite::Error> {
                let mut st = c.prepare_cached(sql)?;
                let rows = st.query_map(
                    params![seeds_json, depth, x.hub_degree, ctx, x.limit as i64],
                    |r| {
                        Ok(Reached {
                            entity: r.get(0)?,
                            score: r.get(1)?,
                            hops: r.get::<_, i64>(2)? as u32,
                        })
                    },
                )?;
                rows.collect()
            })();
            c.progress_handler(0, None::<fn() -> bool>)?;
            match r {
                Ok(reached) => Ok(ExpandResult {
                    reached,
                    timed_out: false,
                }),
                Err(rusqlite::Error::SqliteFailure(e, _))
                    if e.code == rusqlite::ErrorCode::OperationInterrupted =>
                {
                    Ok(ExpandResult {
                        reached: vec![],
                        timed_out: true,
                    })
                }
                Err(e) => Err(e.into()),
            }
        })
    }

    /// Live facts attached to entities (directly or as an edge's evidence) whose scope does not conflict with
    /// `ctx`, scored by the entity score times the fact's confidence (ADR-51 §6 "facts reachable from seeds").
    pub fn facts_of_entities(
        &self,
        scored: &[(i64, f64)],
        ctx: &Value,
        limit: usize,
    ) -> Result<Vec<(Fact, f64)>, StoreError> {
        if scored.is_empty() {
            return Ok(vec![]);
        }
        let ctx = normalize_scope(ctx);
        let mut best: HashMap<i64, f64> = HashMap::new();
        self.db.read(|c| -> Result<(), StoreError> {
            let mut st = c.prepare_cached(
                "SELECT fact_id FROM fact_entity WHERE entity_id = ?1
                 UNION SELECT evidence_fact FROM edge WHERE (src = ?1 OR dst = ?1) AND valid_to IS NULL AND evidence_fact IS NOT NULL",
            )?;
            for (id, score) in scored {
                for f in st.query_map([id], |r| r.get::<_, i64>(0))? {
                    let f = f?;
                    let e = best.entry(f).or_insert(0.0);
                    if *e < *score {
                        *e = *score;
                    }
                }
            }
            Ok(())
        })?;
        let ids: Vec<i64> = best.keys().copied().collect();
        let facts = self.db.read(|c| Self::facts_by_ids(c, &ids))?;
        let mut out: Vec<(Fact, f64)> = facts
            .into_values()
            .filter(|f| f.is_live() && f.weight >= crate::agent::HIDE_BELOW_WEIGHT && !f.untrusted)
            .filter(|f| {
                !matches!(
                    scope_match(&normalize_scope(&f.scope), &ctx),
                    ScopeMatch::Conflict
                )
            })
            .map(|f| {
                let s = best[&f.id] * f.confidence;
                (f, s)
            })
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.id.cmp(&b.0.id)));
        out.truncate(limit);
        Ok(out)
    }

    /// Lowers (or raises) a fact's confidence within 0..1 (reuse signals, ADR-51 §8).
    pub fn adjust_confidence(&self, fact: i64, delta: f64) -> Result<(), StoreError> {
        self.db.write(|tx| {
            tx.execute(
                "UPDATE fact SET confidence = MIN(1.0, MAX(0.0, confidence + ?2)) WHERE id = ?1",
                params![fact, delta],
            )?;
            Ok(())
        })
    }

    /// Marks the usage rows of a run with an outcome signal (`reopened`, `closed`, ...).
    pub fn signal_usage(
        &self,
        run: &str,
        fact: Option<i64>,
        signal: &str,
        outcome: Option<&str>,
    ) -> Result<u64, StoreError> {
        self.db.write(|tx| {
            let n = tx.execute(
                &format!(
                    "UPDATE fact_usage SET signal = ?2, outcome = COALESCE(?3, outcome), outcome_at = {}
                     WHERE run = ?1 AND (?4 IS NULL OR fact = ?4)",
                    now()
                ),
                params![run, signal, outcome, fact],
            )?;
            Ok(n as u64)
        })
    }

    /// Facts shown to a run (for reopen attribution).
    pub fn facts_used_in(&self, run: &str) -> Result<Vec<Fact>, StoreError> {
        let ids: Vec<i64> = self
            .usage_for_run(run)?
            .into_iter()
            .map(|(f, _, _)| f)
            .collect();
        let m = self.db.read(|c| Self::facts_by_ids(c, &ids))?;
        Ok(ids.iter().filter_map(|i| m.get(i).cloned()).collect())
    }
}
