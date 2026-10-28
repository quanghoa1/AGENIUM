//! The graph's rows: entities, edges, candidates, what an expansion reached.

use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub canonical_key: String,
    pub scope: Value,
    pub description: Option<String>,
    pub confidence: f64,
    pub mentions: i64,
    pub degree: i64,
    pub merged_into: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewEntity {
    pub kind: String,
    pub name: String,
    pub scope: Value,
    pub description: Option<String>,
    pub aliases: Vec<String>,
    pub confidence: f64,
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub id: i64,
    pub src: i64,
    pub dst: i64,
    pub kind: String,
    pub scope: Value,
    pub weight: f64,
    pub confidence: f64,
    pub evidence_fact: Option<i64>,
    pub valid_to: Option<String>,
}

/// An entity found by linking, with how it was found and a similarity in 0..1.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub entity: Entity,
    pub via: &'static str,
    pub similarity: f64,
}

/// A node reached by expansion.
#[derive(Debug, Clone, PartialEq)]
pub struct Reached {
    pub entity: i64,
    pub score: f64,
    pub hops: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Expansion {
    pub max_depth: u32,
    pub hub_degree: i64,
    pub limit: usize,
    pub budget: Duration,
}

impl Default for Expansion {
    fn default() -> Self {
        Self {
            max_depth: 2,
            hub_degree: 200,
            limit: 50,
            budget: Duration::from_millis(25),
        }
    }
}

/// Result of an expansion: nodes plus whether the time budget cut it short.
#[derive(Debug, Clone, Default)]
pub struct ExpandResult {
    pub reached: Vec<Reached>,
    pub timed_out: bool,
}

pub(super) const ENTITY_COLS: &str =
    "id, type, name, canonical_key, scope, description, confidence, mentions, degree, merged_into";

pub(super) fn map_entity(r: &rusqlite::Row<'_>) -> rusqlite::Result<(Entity, String)> {
    Ok((
        Entity {
            id: r.get(0)?,
            kind: r.get(1)?,
            name: r.get(2)?,
            canonical_key: r.get(3)?,
            scope: Value::Null,
            description: r.get(5)?,
            confidence: r.get(6)?,
            mentions: r.get(7)?,
            degree: r.get(8)?,
            merged_into: r.get(9)?,
        },
        r.get(4)?,
    ))
}

pub(super) fn decode_entity(v: (Entity, String)) -> Result<Entity, StoreError> {
    let (mut e, scope) = v;
    e.scope = crate::core::from_json(&scope)?;
    Ok(e)
}
