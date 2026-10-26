//! Hybrid retrieval with the graph channel (ADR-51 §7): vector KNN + BM25 + graph expansion, weighted reciprocal
//! rank fusion, scope hard filter before fusion, re-rank by scope x recency x confidence. With no entities linked
//! it is the plain vector + keyword hybrid.

use std::collections::HashMap;

use agenium_store::graph::{normalize_scope, scope_match, Expansion, ScopeMatch};
use agenium_store::{AgentStore, Fact, Via};
use serde_json::Value;

use crate::{Embedder, MemoryError, Recalled};

pub const W_VEC: f64 = 1.0;
pub const W_BM25: f64 = 1.0;
pub const W_GRAPH: f64 = 0.8;
pub const RRF_K: f64 = 60.0;
const POOL: usize = 50;
/// Vector hits farther than this cosine distance are not evidence (similarity < 0.2).
pub const MAX_VEC_DISTANCE: f64 = 0.8;
/// At most this many of the context's rules (proven lessons of its scope) join every recall.
pub const RULES: usize = 3;
/// Entity types whose canonical name becomes a context scope key when the ticket mentions them.
const SCOPE_TYPES: &[&str] = &["vendor", "product", "language", "os", "project"];

/// Where the work happens: the fact `project` column to prefer and the context scope of ADR-51 §5.
#[derive(Debug, Clone, Default)]
pub struct RecallContext {
    pub project: Option<String>,
    pub scope: Value,
}

fn days_since(ts: &str) -> f64 {
    chrono_like_days(ts).unwrap_or(0.0)
}

/// Days between an RFC 3339 timestamp and now.
fn chrono_like_days(ts: &str) -> Option<f64> {
    let then = chrono::DateTime::parse_from_rfc3339(ts).ok()?;
    let secs = chrono::Utc::now().signed_duration_since(then).num_seconds();
    Some(secs.max(0) as f64 / 86_400.0)
}

/// `0.5 + 0.5 * 2^(-age/90)` (ADR-51 §7, floor 0.5).
pub fn recency(updated_at: &str) -> f64 {
    0.5 + 0.5 * 2f64.powf(-days_since(updated_at) / 90.0)
}

pub fn recall(
    store: &AgentStore,
    embedder: &dyn Embedder,
    query: &str,
    ctx: &RecallContext,
    k: usize,
) -> Result<Vec<Recalled>, MemoryError> {
    if k == 0 || store.count_live()? == 0 {
        return Ok(vec![]);
    }
    let qv = embedder.embed_query(query).map_err(MemoryError::Embedder)?;
    let mut vec_hits = store.knn(&qv, POOL)?;
    vec_hits.retain(|(_, d)| *d <= MAX_VEC_DISTANCE);
    let kw_hits = store.keyword(query, POOL)?;

    // Context scope: the caller's, plus scope-like entities the query names and the identity scope of every named
    // entity (a ticket about a Nighthawk router has vendor=netgear even if the caller did not know it). The first
    // value of a key wins.
    let mut scope = normalize_scope(&ctx.scope);
    let linked = store.entities_in_text(query, 10)?;
    if let Some(o) = scope.as_object_mut() {
        for e in &linked {
            if SCOPE_TYPES.contains(&e.kind.as_str()) && !o.contains_key(&e.kind) {
                o.insert(e.kind.clone(), Value::String(e.canonical_key.clone()));
            }
            // A named entity brings its identity scope (the Archer C7 is a TP-Link product).
            for (k, v) in e.scope.as_object().into_iter().flatten() {
                o.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    }

    // Graph channel: query entities (2 hops) and entities of the best text hits (1 hop).
    let mut top: Vec<i64> = Vec::new();
    for (id, _) in vec_hits.iter().take(10).chain(kw_hits.iter().take(10)) {
        if !top.contains(id) {
            top.push(*id);
        }
    }
    let mut fact_seeds: Vec<i64> = Vec::new();
    for f in &top {
        for e in store.fact_entities(*f)? {
            if !fact_seeds.contains(&e) && !linked.iter().any(|l| l.id == e) {
                fact_seeds.push(e);
            }
        }
    }
    let mut scored: Vec<(i64, f64)> = linked.iter().map(|e| (e.id, 1.0)).collect();
    scored.extend(fact_seeds.iter().map(|e| (*e, 0.8)));
    let q_ids: Vec<i64> = linked.iter().map(|e| e.id).collect();
    for (seeds, depth, base) in [(&q_ids, 2, 1.0), (&fact_seeds, 1, 0.8)] {
        if seeds.is_empty() {
            continue;
        }
        let r = store.expand(
            seeds,
            &scope,
            &Expansion {
                max_depth: depth,
                ..Expansion::default()
            },
        )?;
        if r.timed_out {
            tracing::debug!("graph expansion hit its time budget; using the text channels");
        }
        scored.extend(r.reached.iter().map(|n| (n.entity, n.score * base)));
    }
    // The graph proposes what is connected; what is relevant comes first. Ordered by graph score alone, the facts of
    // the entity the query names (hop 0: its deploy day, its latency budget) filled the channel and pushed the
    // two-hop answer out of the top 8 (graph QA: MRR 0.83 -> 0.45 with the graph on, ADR-94). Candidates without a
    // vector rank last.
    let mut graph_hits = store.facts_of_entities(&scored, &scope, POOL * 4)?;
    let ids: Vec<i64> = graph_hits.iter().map(|(f, _)| f.id).collect();
    let sim = store.similarity_to(&qv, &ids)?;
    let relevance = |id: i64| sim.get(&id).copied().unwrap_or(-1.0);
    graph_hits.sort_by(|a, b| {
        relevance(b.0.id)
            .total_cmp(&relevance(a.0.id))
            .then(b.1.total_cmp(&a.1))
            .then(a.0.id.cmp(&b.0.id))
    });
    graph_hits.truncate(POOL);

    // Fusion over the candidates that pass the hard filters.
    let mut fused: HashMap<i64, (f64, [bool; 3])> = HashMap::new();
    let mut add = |id: i64, rank: usize, w: f64, ch: usize| {
        let e = fused.entry(id).or_insert((0.0, [false; 3]));
        e.0 += w / (RRF_K + rank as f64 + 1.0);
        e.1[ch] = true;
    };
    for (r, (id, _)) in vec_hits.iter().enumerate() {
        add(*id, r, W_VEC, 0);
    }
    for (r, (id, _)) in kw_hits.iter().enumerate() {
        add(*id, r, W_BM25, 1);
    }
    for (r, (f, _)) in graph_hits.iter().enumerate() {
        add(f.id, r, W_GRAPH, 2);
    }
    // The graph channel already loaded its facts; only the text hits are read here.
    let mut facts: HashMap<i64, Fact> = graph_hits.into_iter().map(|(f, _)| (f.id, f)).collect();
    let missing: Vec<i64> = fused
        .keys()
        .copied()
        .filter(|id| !facts.contains_key(id))
        .collect();
    for id in missing {
        if let Ok(f) = store.get(id) {
            facts.insert(id, f);
        }
    }
    let mut out: Vec<Recalled> = Vec::new();
    for (id, (rrf, ch)) in fused {
        let Some(f) = facts.remove(&id) else { continue };
        if !f.is_live() || f.untrusted || f.weight < agenium_store::agent::HIDE_BELOW_WEIGHT {
            continue;
        }
        let item_scope = normalize_scope(&f.scope);
        let exact = item_scope
            .as_object()
            .is_none_or(|o| o.iter().all(|(k, v)| scope.get(k) == Some(v)));
        let sf = match scope_match(&item_scope, &scope) {
            ScopeMatch::Conflict => continue,
            ScopeMatch::Factor(x) => {
                let empty = f.scope.as_object().is_none_or(|o| o.is_empty());
                if empty && f.kind == agenium_store::FactKind::Lesson {
                    0.9
                } else {
                    x
                }
            }
        };
        let score = rrf * sf * recency(&f.updated_at) * (0.5 + 0.5 * f.confidence) * f.weight;
        let via = match ch {
            [_, _, true] if !ch[0] && !ch[1] => Via::Graph,
            [true, true, _] | [true, _, true] | [_, true, true] => Via::Hybrid,
            [true, false, false] => Via::Vector,
            _ => Via::Keyword,
        };
        out.push(Recalled {
            id: f.id,
            text: f.text,
            via,
            score,
            project: f.project,
            kind: f.kind,
            cue: f.cue,
            exact,
            validated: f.validated.len(),
            shared: false,
        });
    }
    let project = ctx.project.as_deref();
    out.sort_by(|a, b| {
        let pa = project.is_some() && a.project.as_deref() == project;
        let pb = project.is_some() && b.project.as_deref() == project;
        b.score
            .total_cmp(&a.score)
            .then(pb.cmp(&pa))
            .then(a.id.cmp(&b.id))
    });
    // The context's rules come first, whatever the text channels found: a lesson scoped to this project is one of
    // its rules (decision 35). Lessons are few (signals, two contexts), so this costs little; without it a rule
    // reached a ticket only when the ticket shared its words (the hashing embedder; found on Claude Haiku: a lesson
    // about `deliverables/ or reports/` never reached the ticket for `handbook/`).
    let rules = store.rules_in(&ctx.scope, RULES.min(k))?;
    let mut first: Vec<Recalled> = Vec::new();
    for f in rules {
        match out.iter().position(|r| r.id == f.id) {
            Some(i) => first.push(out.remove(i)),
            None => first.push(Recalled {
                id: f.id,
                via: Via::Rule,
                score: 0.0,
                project: f.project.clone(),
                kind: f.kind,
                cue: f.cue.clone(),
                exact: true,
                validated: f.validated.len(),
                shared: false,
                text: f.text,
            }),
        }
    }
    out.truncate(k - first.len());
    first.append(&mut out);
    Ok(first)
}

/// How a recalled memory is shown to the work (decision 35): a lesson whose scope matches the work exactly is a rule,
/// one from a nearby context a hint to verify first; other facts as they are.
pub fn label(r: &Recalled) -> String {
    let when = r
        .cue
        .as_deref()
        .filter(|c| !c.trim().is_empty())
        .map(|c| format!(" (when: {})", c.trim()))
        .unwrap_or_default();
    // A shared-pool lesson (ADR-87) came from another agent; show it as a hint to verify, never a hard rule.
    if r.shared && r.kind == agenium_store::FactKind::Lesson {
        return format!(
            "Shared lesson (from another agent, verify first): {}{when}",
            r.text
        );
    }
    match (r.kind, r.exact) {
        (agenium_store::FactKind::Lesson, true) => {
            let held = if r.validated > 1 {
                format!(", held in {} contexts", r.validated)
            } else {
                String::new()
            };
            format!("Rule (lesson #{}{held}): {}{when}", r.id, r.text)
        }
        (agenium_store::FactKind::Lesson, false) => {
            format!(
                "Hint from another context (lesson #{}), verify first: {}{when}",
                r.id, r.text
            )
        }
        _ => r.text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recency_curve() {
        assert!(
            (recency("2999-01-01T00:00:00.000Z") - 1.0).abs() < 1e-9,
            "future clamps to now"
        );
        let old = recency("2000-01-01T00:00:00.000Z");
        assert!((0.5..0.51).contains(&old), "{old}");
        assert!(chrono_like_days("1970-01-02T00:00:00.000Z").unwrap() > 20_000.0);
    }
}
