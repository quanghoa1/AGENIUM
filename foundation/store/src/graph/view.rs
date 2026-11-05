//! The whole live graph of an agent's memory, for the user to look at: entities, the edges between them, which facts
//! are about which entity, and the verdicts on pairs of live facts.

use super::*;

/// What the Memory page draws (nothing faded or merged; facts come with the page's own list).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GraphView {
    pub entities: Vec<Entity>,
    pub edges: Vec<Edge>,
    /// `(fact, entity, role)` for live facts.
    pub fact_links: Vec<(i64, i64, String)>,
    /// `(a, b, verdict)` between two live facts: `conflict` (open or decided) or `distinct`.
    pub fact_pairs: Vec<(i64, i64, String)>,
    /// Entities left out past `max_entities` (the least connected).
    pub more: usize,
}

impl AgentStore {
    /// The live graph, the `max_entities` most connected entities and what joins them.
    pub fn graph_view(&self, max_entities: usize) -> Result<GraphView, StoreError> {
        self.db.read(|c| {
            let total: i64 = c.query_row(
                "SELECT count(*) FROM entity WHERE merged_into IS NULL AND valid_to IS NULL",
                [],
                |r| r.get(0),
            )?;
            let mut st = c.prepare(&format!(
                "SELECT {ENTITY_COLS} FROM entity WHERE merged_into IS NULL AND valid_to IS NULL
                 ORDER BY degree + mentions DESC, id LIMIT ?1"
            ))?;
            let entities = st
                .query_map([max_entities as i64], map_entity)?
                .map(|r| {
                    let (mut e, s) = r?;
                    e.scope = crate::core::from_json(&s)?;
                    Ok(e)
                })
                .collect::<Result<Vec<_>, StoreError>>()?;
            let shown: std::collections::HashSet<i64> = entities.iter().map(|e| e.id).collect();
            let mut st = c.prepare(
                "SELECT id, src, dst, type, scope, weight, confidence, evidence_fact, valid_to FROM edge
                 WHERE valid_to IS NULL ORDER BY id",
            )?;
            let edges = st
                .query_map([], |r| {
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
                })?
                .filter_map(|r| match r {
                    Ok((e, _)) if !(shown.contains(&e.src) && shown.contains(&e.dst)) => None,
                    Ok((mut e, s)) => Some(crate::core::from_json(&s).map(|v| {
                        e.scope = v;
                        e
                    })),
                    Err(e) => Some(Err(e.into())),
                })
                .collect::<Result<Vec<_>, StoreError>>()?;
            let live = "SELECT id FROM fact WHERE superseded_by IS NULL";
            let mut st = c.prepare(&format!(
                "SELECT fact_id, entity_id, role FROM fact_entity WHERE fact_id IN ({live}) ORDER BY fact_id, entity_id"
            ))?;
            let fact_links = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .filter(|r| r.as_ref().map_or(true, |(_, e, _)| shown.contains(e)))
                .collect::<Result<Vec<_>, _>>()?;
            let mut st = c.prepare(&format!(
                "SELECT a, b, verdict FROM fact_pair WHERE verdict IN ('conflict', 'distinct')
                 AND a IN ({live}) AND b IN ({live}) ORDER BY a, b"
            ))?;
            let fact_pairs = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(GraphView {
                more: (total as usize).saturating_sub(entities.len()),
                entities,
                edges,
                fact_links,
                fact_pairs,
            })
        })
    }
}
