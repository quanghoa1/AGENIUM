//! Entity resolution (ADR-51 §4): a mention extracted by the model becomes an existing entity or a new one.
//! Order: exact key -> alias -> FTS / vector candidates. Bands (initial values, not calibrated yet): cosine >= 0.90
//! and string-similar -> same entity (the surface form becomes an alias); 0.80..0.90 -> ask the model; below ->
//! new entity. Never across types, never across conflicting identity scopes, never between different versions.

use agenium_store::graph::{
    canonical, normalize_scope, scope_match, Candidate, NewEntity, ScopeMatch,
};
use agenium_store::AgentStore;

use crate::{Embedder, MemoryError};

pub const AUTO_SAME: f64 = 0.90;
pub const ASK_SAME: f64 = 0.80;

/// Digit groups of a name (`firmware 1.2.3` -> ["1", "2", "3"]): different groups = different versions.
fn versions(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in name.chars() {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Token Jaccard >= 0.5 or one canonical name contains the other.
pub fn string_similar(a: &str, b: &str) -> bool {
    let (a, b) = (canonical(a), canonical(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a.contains(&b) || b.contains(&a) {
        return true;
    }
    let ta: std::collections::HashSet<&str> = a.split(' ').collect();
    let tb: std::collections::HashSet<&str> = b.split(' ').collect();
    let inter = ta.intersection(&tb).count() as f64;
    let union = ta.union(&tb).count() as f64;
    inter / union >= 0.5
}

/// `firmware 1.2` vs `firmware 1.3`: the same words with different numbers name different versions.
pub fn other_version(a: &str, b: &str) -> bool {
    let words = |s: &str| -> String {
        canonical(s)
            .chars()
            .filter(|c| !c.is_ascii_digit())
            .filter(|c| !c.is_whitespace())
            .collect()
    };
    words(a) == words(b) && versions(a) != versions(b)
}

/// Hard rules: may `cand` be the entity the mention names? Exact and alias hits are trusted on names; scopes must
/// never conflict; similar-looking names that differ only in a version number are different entities.
pub fn compatible(mention: &NewEntity, cand: &Candidate) -> bool {
    if matches!(
        scope_match(&normalize_scope(&mention.scope), &cand.entity.scope),
        ScopeMatch::Conflict
    ) {
        return false;
    }
    matches!(cand.via, "exact" | "alias") || !other_version(&mention.name, &cand.entity.name)
}

/// What to do with a mention.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Same(i64, f64),
    /// Ambiguous band: the model decides.
    Ask(i64, f64),
    New,
}

pub fn decide(mention: &NewEntity, cands: &[Candidate]) -> Decision {
    for c in cands.iter().filter(|c| compatible(mention, c)) {
        match c.via {
            "exact" | "alias" => return Decision::Same(c.entity.id, c.similarity),
            _ if c.similarity >= AUTO_SAME && string_similar(&mention.name, &c.entity.name) => {
                return Decision::Same(c.entity.id, c.similarity)
            }
            _ => {}
        }
    }
    cands
        .iter()
        .filter(|c| compatible(mention, c) && c.similarity >= ASK_SAME)
        .max_by(|a, b| a.similarity.total_cmp(&b.similarity))
        .map(|c| Decision::Ask(c.entity.id, c.similarity))
        .unwrap_or(Decision::New)
}

pub fn entity_text(name: &str, description: Option<&str>) -> String {
    format!("{name}. {}", description.unwrap_or(""))
}

/// Links a mention to an existing entity (recording the alias and the mention) or creates it. Returns
/// `(id, created)`; a "new" mention whose exact identity appeared earlier in the same episode is linked.
pub fn link_or_create(
    store: &AgentStore,
    embedder: &dyn Embedder,
    mention: &NewEntity,
    same: Option<i64>,
) -> Result<(i64, bool), MemoryError> {
    if let Some(id) = same {
        let e = store.entity(id)?;
        // Upsert with the existing identity records the mention and the surface form as an alias.
        let (got, _) = store.upsert_entity(
            &NewEntity {
                kind: e.kind.clone(),
                name: e.name.clone(),
                scope: e.scope.clone(),
                description: mention.description.clone(),
                aliases: vec![mention.name.clone()],
                confidence: mention.confidence,
                evidence: mention.evidence.clone(),
            },
            None,
        )?;
        return Ok((got, false));
    }
    let v = embedder
        .embed(&[entity_text(&mention.name, mention.description.as_deref())])
        .map_err(MemoryError::Embedder)?
        .pop();
    Ok(store.upsert_entity(mention, v.as_deref())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agenium_store::graph::Entity;
    use serde_json::json;

    fn cand(
        id: i64,
        name: &str,
        scope: serde_json::Value,
        via: &'static str,
        sim: f64,
    ) -> Candidate {
        Candidate {
            entity: Entity {
                id,
                kind: "product".into(),
                name: name.into(),
                canonical_key: canonical(name),
                scope: normalize_scope(&scope),
                description: None,
                confidence: 0.5,
                mentions: 1,
                degree: 0,
                merged_into: None,
            },
            via,
            similarity: sim,
        }
    }

    fn m(name: &str, scope: serde_json::Value) -> NewEntity {
        NewEntity {
            kind: "product".into(),
            name: name.into(),
            scope,
            description: None,
            aliases: vec![],
            confidence: 0.5,
            evidence: None,
        }
    }

    #[test]
    fn bands_and_hard_rules() {
        assert!(string_similar("Archer C7 router", "archer c7"));
        assert!(!string_similar("Archer C7", "Nighthawk R7000"));
        assert_eq!(
            decide(
                &m("AC1750", json!({})),
                &[cand(1, "Archer C7", json!({}), "alias", 0.99)]
            ),
            Decision::Same(1, 0.99)
        );
        assert_eq!(
            decide(
                &m("Archer-C7 router", json!({})),
                &[cand(1, "Archer C7", json!({}), "vector", 0.93)]
            ),
            Decision::Same(1, 0.93)
        );
        assert_eq!(
            decide(
                &m("TL router", json!({})),
                &[cand(1, "Archer C7", json!({}), "vector", 0.85)]
            ),
            Decision::Ask(1, 0.85)
        );
        assert_eq!(
            decide(
                &m("Something", json!({})),
                &[cand(1, "Archer C7", json!({}), "vector", 0.5)]
            ),
            Decision::New
        );
        // Vendor conflict and version differences never merge, even on exact name.
        assert_eq!(
            decide(
                &m("Router", json!({"vendor": "netgear"})),
                &[cand(
                    1,
                    "Router",
                    json!({"vendor": "tp-link"}),
                    "exact",
                    1.0
                )]
            ),
            Decision::New
        );
        assert_eq!(
            decide(
                &m("firmware 1.3", json!({})),
                &[cand(1, "firmware 1.2", json!({}), "vector", 0.97)]
            ),
            Decision::New
        );
    }
}
