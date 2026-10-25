//! Entity resolution (ADR-51 §4): a mention extracted by the model becomes an existing entity or a new one.
//! Order: exact key -> alias -> FTS / vector candidates. Bands: cosine >= 0.90 and the same name -> same entity (the
//! surface form becomes an alias); 0.80..0.90 -> ask the model; below -> new entity. Never across types, never across
//! conflicting identity scopes, never between different versions. Measured on 2 152 labelled mentions
//! (`examples/entity_resolution.rs`, ADR-94).

use agenium_store::graph::{
    canonical, normalize_scope, scope_match, Candidate, Entity, NewEntity, ScopeMatch,
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

/// Words that never tell two things apart when added to a name ("the Redis server", "Archer C7 device").
const GENERIC: &[&str] = &[
    "the", "a", "an", "our", "my", "this", "that", "service", "server", "app", "application", "tool", "library", "lib",
    "framework", "system", "device", "instance", "setup", "cluster", "database", "db", "api", "platform", "engine",
    "software", "project", "module", "package", "product",
];

/// Do two names say the same thing? Equal once spaces are ignored ("ArcherC7"), or every word one has and the other
/// lacks tells nothing apart: a generic word, or a word of `context` (what is known about the stored entity: its
/// description, type and identity scope, so "TP-Link Archer C7 router" names the Archer C7 described as a TP-Link
/// router). A qualifier ("Pro", "Galera", "R7000P", "Webpacker") keeps the names apart. Substring containment and
/// token overlap, used before, merged siblings such as "Pixel 8 Pro" into "Pixel 8" (ADR-94).
pub fn same_name(a: &str, b: &str, context: &[String]) -> bool {
    let (a, b) = (canonical(a), canonical(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a.replace(' ', "") == b.replace(' ', "") {
        return true;
    }
    let ta: std::collections::BTreeSet<&str> = a.split(' ').collect();
    let tb: std::collections::BTreeSet<&str> = b.split(' ').collect();
    let noise = |w: &&str| GENERIC.contains(w) || context.iter().any(|c| c == w);
    ta.intersection(&tb).any(|w| !noise(w)) && ta.symmetric_difference(&tb).all(noise)
}

/// The words known about a stored entity that may appear around its name: description, type and identity scope.
pub fn context_words(e: &Entity) -> Vec<String> {
    let mut text = format!("{} {}", e.description.as_deref().unwrap_or(""), e.kind);
    for v in e.scope.as_object().into_iter().flat_map(|o| o.values()) {
        text.push(' ');
        text.push_str(v.as_str().unwrap_or(""));
    }
    canonical(&text).split(' ').filter(|w| !w.is_empty()).map(str::to_owned).collect()
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
            _ if c.similarity >= AUTO_SAME
                && same_name(&mention.name, &c.entity.name, &context_words(&c.entity)) =>
            {
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

/// The text an entity's vector is made from: its name alone. Descriptions vary with every episode that mentions the
/// entity, and comparing a bare mention with "name. description" hid the name: on 2 152 labelled mentions, name
/// vectors raised resolution recall from 0.75 to 0.92 with precision 0.996 (ADR-94). Descriptions still reach the
/// model in the ambiguous band (`pipeline::same_line`).
pub fn entity_text(name: &str) -> String {
    format!("{}. ", name.trim())
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
        .embed(&[entity_text(&mention.name)])
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
        assert!(same_name("Archer C7 router", "archer c7", &["router".to_owned()]));
        assert!(!same_name("Archer C7 router", "archer c7", &[]), "an unexplained extra word goes to the model");
        assert!(same_name("ArcherC7", "Archer-C7", &[]));
        assert!(!same_name("Archer C7", "Nighthawk R7000", &[]));
        // Qualifiers keep siblings apart; words the stored entity is described with do not.
        let ctx: Vec<String> = ["google", "android", "phone"].map(String::from).to_vec();
        assert!(!same_name("Pixel 8 Pro phone", "Pixel 8", &ctx));
        assert!(same_name("Google Pixel 8 phone", "Pixel 8", &ctx));
        assert!(!same_name("Webpacker", "Webpack", &[]));
        assert!(!same_name("the service", "the api", &[]));
        assert_eq!(
            decide(
                &m("AC1750", json!({})),
                &[cand(1, "Archer C7", json!({}), "alias", 0.99)]
            ),
            Decision::Same(1, 0.99)
        );
        let mut described = cand(1, "Archer C7", json!({"vendor": "tp-link"}), "vector", 0.93);
        described.entity.description = Some("dual-band Wi-Fi router".into());
        assert_eq!(
            decide(&m("TP-Link Archer-C7 router", json!({})), &[described.clone()]),
            Decision::Same(1, 0.93)
        );
        // A qualifier the entity is not described with: the model decides, even at 0.93.
        assert_eq!(
            decide(&m("Archer C7 Pro", json!({})), &[described]),
            Decision::Ask(1, 0.93)
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
