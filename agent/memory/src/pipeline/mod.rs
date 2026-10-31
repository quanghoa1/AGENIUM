//! Memory pipeline (decision 2, "Memory riêng tư"; graph part ADR-51 §3/§8): after a ticket is Closed or Reopened
//! the agent's own model (1) extracts short English facts with their scope, entities and relations, (2) each fact
//! is matched against the agent's memory by hybrid search, (3) the model decides ADD / UPDATE / DELETE / NOOP per
//! fact, (4) the decisions are applied (UPDATE keeps history through `superseded_by`, DELETE retires; nothing is
//! erased), (5) entities are resolved (`resolve`) and linked, relations become edges with the fact as evidence,
//! (6) reuse signals: facts shown to the run get `helped` when the ticket closed; on a reopen the model names the
//! memories that misled it and they lose confidence.
//!
//! Guards: facts are redacted and dropped when they look like secrets; ids the model names must be among the ones
//! it was shown; facts from external content (web, MCP, files) are stored untrusted, never become lessons and never
//! create `fixes` / `workaround_for` / `applies_to` edges; a lesson without scope is scoped to the project.

use std::collections::{BTreeMap, BTreeSet};

use agenium_core::Effort;
use agenium_llm::{ChatRequest, Message, Provider, RetryPolicy, SystemBlock, Usage};
use agenium_store::graph::{canonical, NewEntity};
use agenium_store::{AgentStore, FactKind, NewFact};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::resolve::{self, Decision};
use crate::{Embedder, MemoryError};

mod types;

pub use parse::{complete_items, ephemeral, json_object};
use parse::{is_empty_scope, kind_of};
use rules::DECIDE_RULES;
pub use rules::{same_line, EXTRACT_RULES, SAME_RULES};
pub use types::*;
mod parse;
mod rules;

/// Max facts kept from one episode.
pub const MAX_FACTS: usize = 12;
/// Existing facts shown to the model per new fact.
pub const CANDIDATES: usize = 5;
pub const MAX_FACT_CHARS: usize = 400;
/// Confidence change of a memory the model says misled a reopened run / of one used in a closed ticket.
pub const HURT_DELTA: f64 = -0.15;
pub const HELPED_DELTA: f64 = 0.05;
/// Relation types untrusted content may not create.
const TRUSTED_ONLY_EDGES: &[&str] = &["fixes", "workaround_for", "applies_to"];
const EDGE_TYPES: &[&str] = &[
    "is_a",
    "part_of",
    "depends_on",
    "uses",
    "configured_with",
    "causes",
    "fixes",
    "workaround_for",
    "conflicts_with",
    "applies_to",
    "replaced_by",
    "related_to",
];

/// How sure a new memory is, by what backs it: the user's own preference in the chat most, a fact from work the review
/// accepted more than one from work it reopened, content read from outside least. Use moves it from there
/// ([`HELPED_DELTA`], [`HURT_DELTA`]); a flat start made every memory read 50%, telling the user nothing.
pub fn initial_confidence(kind: FactKind, external: bool, ep: &Episode) -> f64 {
    if external {
        0.3
    } else if ep.source.starts_with("chat:") {
        if kind == FactKind::Preference {
            0.7
        } else {
            0.6
        }
    } else if ep.reopened {
        0.45
    } else {
        0.6
    }
}

async fn ask(
    provider: &dyn Provider,
    m: &PipelineModel,
    rules: &str,
    user: String,
    report: &mut LearnReport,
) -> Result<Value, MemoryError> {
    let req = ChatRequest {
        model: m.model.clone(),
        system: vec![SystemBlock {
            text: rules.to_owned(),
            cache: false,
        }],
        messages: vec![Message::user_text(user)],
        tools: vec![],
        effort: m.effort.clone(),
        max_tokens: m.max_tokens,
        cache_key: None,
        long_cache: false,
        tools_off: false,
    };
    // An answer that is empty or not JSON is asked for once more: a cheap model sometimes answers nothing at all
    // (DeepSeek, the shared-pool eval of 2026-10-31), and the whole episode was lost to it.
    let mut text = String::new();
    for _ in 0..2 {
        let resp = m
            .retry
            .run(
                |_| provider.chat(&req, &|_| {}),
                tokio::time::sleep,
                |_, _, _| {},
            )
            .await
            .map_err(|e| MemoryError::Model(e.to_string()))?;
        report.tokens_in += resp.usage.total_input();
        report.tokens_out += resp.usage.output;
        report.usage.add(&resp.usage);
        text = resp.message.text();
        if let Some(v) = json_object(&text) {
            return Ok(v);
        }
        // An extraction cut off by the token budget still holds every fact it finished: keep those rather than lose
        // the whole episode (2026-10-25: DeepSeek's answer for a ticket stopped mid-fact and the ticket taught nothing).
        if let Some(facts) = complete_items(&text, "facts") {
            report.rejected.push(format!(
                "the extraction answer was cut off; kept its {} complete fact(s)",
                facts.len()
            ));
            return Ok(serde_json::json!({ "facts": facts }));
        }
    }
    Err(MemoryError::Model(format!(
        "the answer is not JSON: {}",
        text.chars().take(200).collect::<String>()
    )))
}

/// A memory id the model named: a number, or the same written as text (`"2"`, `"id=2"`). Gemini Flash answered
/// `["2"]` for the shared lesson that misled the work, and the blame was lost (the shared-pool eval, 2026-10-31).
fn memory_id(x: &Value) -> Option<i64> {
    x.as_i64().or_else(|| {
        let s = x.as_str()?.trim();
        s.strip_prefix("id=").unwrap_or(s).trim().parse().ok()
    })
}

/// Forgetting and consolidation after each episode (decision 35): weights decay while facts go unused, and copies of
/// the same fact fold into one.
fn maintain(store: &AgentStore, report: &mut LearnReport) -> Result<(), MemoryError> {
    report.decayed = store.decay(
        chrono::Utc::now(),
        agenium_store::agent::DECAY_HALF_LIFE_DAYS,
    )?;
    report.consolidated = store.consolidate()?;
    Ok(())
}

/// Runs the pipeline for one episode. `clean` redacts known secret values and returns `None` when the text
/// still looks like a secret (the fact is dropped).
pub async fn learn(
    provider: &dyn Provider,
    model: &PipelineModel,
    store: &AgentStore,
    embedder: &dyn Embedder,
    ep: &Episode,
    clean: &(dyn Fn(&str) -> Option<String> + Send + Sync),
) -> Result<LearnReport, MemoryError> {
    let mut report = LearnReport::default();
    let transcript: String = ep.transcript.chars().take(60_000).collect();
    let mut prompt = format!(
        "Episode {} (outcome: {})\nSIGNALS: {}\n\n{transcript}",
        ep.source,
        ep.outcome,
        if ep.signals.is_empty() {
            "none (no lessons)".to_owned()
        } else {
            ep.signals.join("; ")
        }
    );
    // The pool's lessons are named past every id of the agent's own shown (a pool id may equal one of them): an id the
    // model names is then one memory, and a pooled lesson's blame never lands on the agent's own (audit M3).
    let first_shared = ep.used.iter().map(|(id, _)| *id).max().unwrap_or(0).max(0) + 1;
    if !ep.used.is_empty() || !ep.shared_used.is_empty() {
        prompt.push_str("\n\n## MEMORIES SHOWN to this run\n");
        for (id, t) in &ep.used {
            prompt.push_str(&format!("id={id}: {t}\n"));
        }
        for (i, (_, t)) in ep.shared_used.iter().enumerate() {
            prompt.push_str(&format!(
                "id={}: (a shared lesson from another agent) {t}\n",
                first_shared + i as i64
            ));
        }
        // Next to the list it is about: at the end of the long rules, a cheap model missed most culprits and named
        // memories the work never acted on (the shared-pool eval, 2026-10-31).
        if ep.reopened {
            prompt.push_str(
                "The review reopened this work. In \"misleading\", list the id of each memory above that the \
                 rejected work followed where the review says it was wrong, whether the work quoted it or not. \
                 Leave out a memory the work did not act on, even one that is wrong here. If none led to the \
                 rejection, \"misleading\" is [].\n",
            );
        }
    }
    let v = ask(provider, model, EXTRACT_RULES, prompt, &mut report).await?;
    let raw: Vec<Extracted> = serde_json::from_value(v["facts"].clone()).unwrap_or_default();
    let misleading: Vec<i64> = v["misleading"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(memory_id)
        .collect();
    let mut facts: Vec<Extracted> = Vec::new();
    for mut f in raw.into_iter().take(MAX_FACTS) {
        let t = f.text.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(what) = ephemeral(t) {
            report
                .rejected
                .push(format!("an episode detail ({what}), not durable knowledge"));
            continue;
        }
        match clean(t) {
            Some(c) => {
                f.text = c.chars().take(MAX_FACT_CHARS).collect();
                if f.external && f.kind.as_deref() == Some("lesson") {
                    f.kind = Some("fact".into());
                    report
                        .rejected
                        .push("an external lesson was stored as an untrusted fact".into());
                }
                if f.kind.as_deref() == Some("lesson") {
                    if ep.signals.is_empty() {
                        report
                            .rejected
                            .push("a lesson from an episode without signals".into());
                        continue;
                    }
                    if f.cue.as_deref().is_none_or(|c| c.trim().is_empty()) {
                        report.rejected.push("a lesson without a cue".into());
                        continue;
                    }
                }
                if f.kind.as_deref() == Some("lesson") && is_empty_scope(&f.scope) {
                    if let Some(p) = &ep.project_name {
                        f.scope = serde_json::json!({ "project": p });
                    }
                }
                facts.push(f);
            }
            None => report
                .rejected
                .push("a fact that looks like a secret".into()),
        }
    }
    report.extracted = facts.len();

    // Reuse signals (ADR-51 §8): misleading memories of a reopened run lose confidence; on a clean close the
    // memories shown were helpful.
    if let Some(run) = &ep.run {
        let shown: BTreeSet<i64> = ep.used.iter().map(|(id, _)| *id).collect();
        if ep.reopened {
            for id in misleading.iter().filter(|id| shown.contains(id)) {
                store.adjust_confidence(*id, HURT_DELTA)?;
                store.signal_usage(run, Some(*id), "reopened", Some("hurt"))?;
                report.misleading.push(*id);
            }
            // The shared lessons named are only reported: the pool weighs them, this agent's memory is not theirs.
            let named: BTreeSet<i64> = misleading.iter().copied().collect();
            report.misleading_shared = named
                .iter()
                .filter_map(|id| id.checked_sub(first_shared))
                .filter_map(|i| usize::try_from(i).ok())
                .filter_map(|i| ep.shared_used.get(i).map(|(pool_id, _)| *pool_id))
                .collect();
        } else if !shown.is_empty() {
            // A lesson that held again counts the context (decision 35: two make it a skill-update candidate). Only
            // one that was in play does (ADR-118): shown as a rule of this context and found by the work's own words.
            // A hint from another context, or a project rule put before every run, says nothing by a quiet close.
            let context = format!("{}/{}", ep.project.as_deref().unwrap_or("-"), ep.source);
            let in_play = store.lessons_in_play(run)?;
            for id in &shown {
                store.adjust_confidence(*id, HELPED_DELTA)?;
                // The ticket a lesson was learned on is not a second context: a lesson holds only where it was
                // applied afresh (found by the learning chain: a lesson counted the ticket it came from).
                if in_play.contains(id)
                    && store.get(*id).is_ok_and(|f| {
                        f.kind == FactKind::Lesson
                            && !(f.source == ep.source && f.project == ep.project)
                    })
                {
                    store.validate_in(*id, &context)?;
                    report.validated += 1;
                }
            }
            report.helped = store.signal_usage(run, None, "closed", Some("helped"))? as usize;
        }
    }
    if facts.is_empty() {
        maintain(store, &mut report)?;
        return Ok(report);
    }

    // Match each new fact against memory.
    let texts: Vec<String> = facts.iter().map(|f| f.text.clone()).collect();
    let vectors = crate::cpu_bound(|| embedder.embed(&texts)).map_err(MemoryError::Embedder)?;
    let mut shown: BTreeSet<i64> = BTreeSet::new();
    // The memories shown under each new fact.
    let mut shown_for: Vec<BTreeSet<i64>> = vec![BTreeSet::new(); facts.len()];
    let mut prompt = String::new();
    for (i, (f, v)) in facts.iter().zip(&vectors).enumerate() {
        prompt.push_str(&format!("NEW {i}: {}\n", f.text));
        let hits = store.hybrid(&f.text, Some(v), CANDIDATES)?;
        if hits.is_empty() {
            prompt.push_str("  (no similar memories)\n");
        }
        for h in hits {
            shown.insert(h.fact.id);
            shown_for[i].insert(h.fact.id);
            prompt.push_str(&format!("  EXISTING id={}: {}\n", h.fact.id, h.fact.text));
        }
    }
    let ops: Vec<Op> = if shown.is_empty() {
        (0..facts.len())
            .map(|i| Op::Add {
                fact: i,
                text: None,
            })
            .collect()
    } else {
        let v = match ask(provider, model, DECIDE_RULES, prompt, &mut report).await {
            Ok(v) => v,
            // The episode's reuse signals are recorded by now (a misleading memory of the agent's lost confidence):
            // failing the whole episode here lost the shared lessons' blame, and a retry would count the agent's
            // twice. The new facts are what is lost, said in the report.
            Err(e) => {
                report.failed = Some(format!("the new facts were not stored: {e}"));
                maintain(store, &mut report)?;
                return Ok(report);
            }
        };
        v["ops"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|o| match serde_json::from_value::<Op>(o.clone()) {
                Ok(op) => Some(op),
                Err(e) => {
                    report
                        .rejected
                        .push(format!("unreadable operation {o}: {e}"));
                    None
                }
            })
            .collect()
    };

    let new_fact = |i: usize, text: Option<String>| -> Option<(NewFact, Vec<f32>)> {
        let f = facts.get(i)?;
        let text = match text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()) {
            Some(t) => clean(&t)?.chars().take(MAX_FACT_CHARS).collect(),
            None => f.text.clone(),
        };
        let v = if text == f.text {
            vectors[i].clone()
        } else {
            embedder.embed(std::slice::from_ref(&text)).ok()?.pop()?
        };
        Some((
            NewFact {
                text,
                kind: kind_of(f.kind.as_deref()),
                project: ep.project.clone(),
                scope: agenium_store::graph::normalize_scope(&f.scope),
                cue: f
                    .cue
                    .as_deref()
                    .map(str::trim)
                    .filter(|c| !c.is_empty())
                    .and_then(clean)
                    .map(|c| c.chars().take(MAX_FACT_CHARS).collect()),
                evidence: Some(ep.outcome.chars().take(200).collect()),
                source: ep.source.clone(),
                untrusted: f.external,
                confidence: initial_confidence(kind_of(f.kind.as_deref()), f.external, ep),
            },
            v,
        ))
    };
    // External content (web, MCP, files) neither rewrites nor retires trusted memory: what a file says can be stale or
    // planted (found on Claude Haiku: a project file read as "nothing enforces this" rewrote a proven lesson into an
    // untrusted one and retired its twin, so the next ticket was not reminded of the rule). It is kept beside it.
    let trusted = |id: i64| store.get(id).is_ok_and(|o| !o.untrusted);
    // A DELETE retires trusted memory on trusted evidence only: the whole episode is when nothing in it is external;
    // otherwise the DELETE names the new fact that contradicts the memory, and that fact must be trusted and have been
    // shown the memory (review of 2026-10-27: one trusted fact in the episode unlocked every DELETE, those an
    // external fact asked for included).
    let all_trusted = facts.iter().all(|f| !f.external);
    let delete_evidence = |id: i64, fact: Option<usize>| {
        all_trusted
            || fact.is_some_and(|i| {
                facts.get(i).is_some_and(|f| !f.external) && shown_for[i].contains(&id)
            })
    };
    let mut touched: BTreeSet<i64> = BTreeSet::new();
    let mut stored: Vec<(usize, i64)> = Vec::new();
    for op in ops {
        match op {
            Op::Add { fact, text } => match new_fact(fact, text) {
                Some((nf, v)) => {
                    stored.push((fact, store.add(&nf, Some(&v))?));
                    report.added += 1;
                    report.changes.push(Change {
                        op: "add",
                        kind: nf.kind,
                        text: nf.text,
                        was: None,
                    });
                }
                None => report
                    .rejected
                    .push(format!("ADD of unknown or unsafe fact {fact}")),
            },
            Op::Update { fact, id, text } => {
                if !shown.contains(&id) || !touched.insert(id) {
                    report
                        .rejected
                        .push(format!("UPDATE of id {id} that was not shown (or twice)"));
                    continue;
                }
                match new_fact(fact, text) {
                    Some((nf, v)) if nf.untrusted && trusted(id) => {
                        stored.push((fact, store.add(&nf, Some(&v))?));
                        report.added += 1;
                        report.changes.push(Change {
                            op: "add",
                            kind: nf.kind,
                            text: nf.text.clone(),
                            was: None,
                        });
                        report.rejected.push(format!(
                            "external content may not rewrite trusted memory {id}; stored beside it as untrusted"
                        ));
                    }
                    Some((mut nf, v)) => {
                        // A lesson refined by a later episode stays a lesson: its kind, cue, trust and the contexts it
                        // held in carry over (a clean episode yields only facts, so its merged text would otherwise
                        // turn a proven lesson into a plain fact and reset what it earned; found on Claude Haiku).
                        let was = store.get(id).ok();
                        let old = was.clone().filter(|o| o.kind == FactKind::Lesson);
                        if let Some(o) = &old {
                            nf.kind = FactKind::Lesson;
                            nf.cue = nf.cue.or_else(|| o.cue.clone());
                            nf.confidence = nf.confidence.max(o.confidence);
                        }
                        let new_id = store.supersede(id, &nf, Some(&v))?;
                        for context in old.iter().flat_map(|o| o.validated.iter()) {
                            store.validate_in(new_id, context)?;
                        }
                        stored.push((fact, new_id));
                        report.updated += 1;
                        report.changes.push(Change {
                            op: "update",
                            kind: nf.kind,
                            text: nf.text,
                            was: was.map(|o| o.text),
                        });
                    }
                    None => report
                        .rejected
                        .push(format!("UPDATE with unknown or unsafe fact {fact}")),
                }
            }
            Op::Delete { id, fact } => {
                if !shown.contains(&id) || !touched.insert(id) {
                    report
                        .rejected
                        .push(format!("DELETE of id {id} that was not shown (or twice)"));
                    continue;
                }
                if !delete_evidence(id, fact) && trusted(id) {
                    report.rejected.push(format!(
                        "DELETE of trusted memory {id} without a trusted new fact that contradicts it"
                    ));
                    continue;
                }
                // A ticket closed without a signal (no reopen, failed merge or correction) confirms the work; it does
                // not disprove a lesson. Found on DeepSeek (ADR-146): a clean ticket deleted the lesson that had kept
                // it right, and the next ticket, no longer reminded, was reopened. Refining it is an UPDATE.
                if ep.signals.is_empty() && store.get(id).is_ok_and(|o| o.kind == FactKind::Lesson)
                {
                    report.rejected.push(format!(
                        "DELETE of lesson {id} by an episode with no signal: a clean ticket does not disprove a \
                         lesson (UPDATE refines it)"
                    ));
                    continue;
                }
                let gone = store.get(id).ok();
                store.retire(id)?;
                report.deleted += 1;
                if let Some(o) = gone {
                    report.changes.push(Change {
                        op: "delete",
                        kind: o.kind,
                        text: o.text,
                        was: None,
                    });
                }
            }
            Op::Noop { .. } => report.noop += 1,
        }
    }

    // Entities and relations of the stored facts.
    let mut decisions: Vec<(usize, i64, usize, NewEntity, Decision)> = Vec::new(); // (fact idx, fact id, mention idx, ...)
    for (fi, fid) in &stored {
        let f = &facts[*fi];
        for (mi, m) in f.entities.iter().enumerate() {
            let Some(name) = clean(m.name.trim()).filter(|n| !n.is_empty()) else {
                continue;
            };
            let kind = if m.kind.trim().is_empty() {
                "concept".to_owned()
            } else {
                m.kind.clone()
            };
            let mention = NewEntity {
                kind,
                name,
                scope: m.scope.clone(),
                description: m.description.clone(),
                aliases: vec![],
                confidence: if f.external { 0.3 } else { 0.6 },
                evidence: Some(ep.source.clone()),
            };
            let v = embedder
                .embed(&[resolve::entity_text(&mention.name)])
                .map_err(MemoryError::Embedder)?
                .pop();
            let cands = store.entity_candidates(&mention.kind, &mention.name, v.as_deref(), 5)?;
            let d = resolve::decide(&mention, &cands);
            decisions.push((*fi, *fid, mi, mention, d));
        }
    }
    // One model call for every ambiguous pair.
    let asks: Vec<usize> = decisions
        .iter()
        .enumerate()
        .filter(|(_, d)| matches!(d.4, Decision::Ask(..)))
        .map(|(i, _)| i)
        .collect();
    if !asks.is_empty() {
        let mut q = String::new();
        for (n, i) in asks.iter().enumerate() {
            let (_, _, _, m, d) = &decisions[*i];
            let Decision::Ask(id, sim) = d else { continue };
            let e = store.entity(*id)?;
            q.push_str(&same_line(n, m, &e, *sim));
        }
        let mut answers: Vec<bool> = match ask(provider, model, SAME_RULES, q, &mut report).await {
            Ok(v) => v["same"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|b| b.as_bool().unwrap_or(false))
                .collect(),
            Err(e) => {
                report.rejected.push(format!(
                    "entity resolution answer unusable ({e}); treated as different"
                ));
                vec![]
            }
        };
        // Verdicts are positional: one missing in the middle would shift every later one onto the wrong pair (DeepSeek
        // answered 9 verdicts for 10 pairs in 4 of 57 groups, ADR-94). A miscount trusts none of them.
        if !answers.is_empty() && answers.len() != asks.len() {
            report.rejected.push(format!(
                "entity resolution answer has {} verdicts for {} pairs; treated as different",
                answers.len(),
                asks.len()
            ));
            answers.clear();
        }
        for (n, i) in asks.iter().enumerate() {
            let same = answers.get(n).copied().unwrap_or(false);
            if let Decision::Ask(id, sim) = decisions[*i].4 {
                decisions[*i].4 = if same {
                    Decision::Same(id, sim)
                } else {
                    Decision::New
                };
            }
        }
    }
    let mut ids: BTreeMap<(usize, String), i64> = BTreeMap::new();
    for (fi, fid, mi, mention, d) in &decisions {
        let same = match d {
            Decision::Same(id, _) => Some(*id),
            _ => None,
        };
        let (eid, created) = resolve::link_or_create(store, embedder, mention, same)?;
        if created {
            report.entities_new += 1;
        } else {
            report.entities_linked += 1;
        }
        let role = facts[*fi].entities[*mi]
            .role
            .as_deref()
            .unwrap_or("mention");
        store.link_fact(*fid, eid, role)?;
        ids.insert((*fi, canonical(&mention.name)), eid);
        ids.insert((*fi, canonical(&facts[*fi].entities[*mi].name)), eid);
    }
    for (fi, fid) in &stored {
        let f = &facts[*fi];
        for r in &f.relations {
            let kind = r.kind.trim().to_ascii_lowercase();
            if !EDGE_TYPES.contains(&kind.as_str()) {
                report
                    .rejected
                    .push(format!("unknown relation type {kind:?}"));
                continue;
            }
            if f.external && TRUSTED_ONLY_EDGES.contains(&kind.as_str()) {
                report
                    .rejected
                    .push(format!("external content may not create a {kind} relation"));
                continue;
            }
            let (Some(s), Some(d)) = (
                ids.get(&(*fi, canonical(&r.src))),
                ids.get(&(*fi, canonical(&r.dst))),
            ) else {
                report.rejected.push(format!(
                    "relation {} -{kind}-> {} names an entity the fact does not list",
                    r.src, r.dst
                ));
                continue;
            };
            if s == d {
                continue;
            }
            let scope = if is_empty_scope(&r.scope) {
                &f.scope
            } else {
                &r.scope
            };
            let conf = if f.external { 0.3 } else { 0.6 };
            store.add_edge(*s, *d, &kind, scope, conf, Some(*fid), "extract")?;
            report.edges += 1;
        }
    }
    maintain(store, &mut report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    /// A memory id the model writes as text still names the memory; anything else names none.
    #[test]
    fn a_memory_id_written_as_text_is_read() {
        use serde_json::json;
        let ids: Vec<Option<i64>> = [
            json!(2),
            json!("2"),
            json!(" id=7 "),
            json!("two"),
            json!(2.5),
            json!(null),
        ]
        .iter()
        .map(super::memory_id)
        .collect();
        assert_eq!(ids, vec![Some(2), Some(2), Some(7), None, None, None]);
    }

    /// An extraction answer cut off mid-fact keeps the facts it finished (braces and escapes inside strings do not
    /// confuse it); an answer without a single finished fact keeps nothing.
    #[test]
    fn a_cut_off_extraction_keeps_its_complete_facts() {
        use super::complete_items;
        let cut = r#"{"facts": [{"text": "Owner line {first} is \"Owner: docs-team\".", "kind": "lesson", "entities": [{"name": "docs"}]}, {"text": "Second fact", "kind": "fact"}, {"text": "Third fact is cut off at "#;
        let kept = complete_items(cut, "facts").unwrap();
        assert_eq!(kept.len(), 2);
        assert_eq!(
            kept[0]["text"],
            "Owner line {first} is \"Owner: docs-team\"."
        );
        assert_eq!(kept[1]["text"], "Second fact");
        assert!(complete_items(r#"{"facts": [{"text": "cut"#, "facts").is_none());
        assert!(complete_items("no json at all", "facts").is_none());
    }

    /// The noisy facts the 2026-10-25 memory-reuse run extracted are refused; the useful ones from the same run pass.
    #[test]
    fn episode_details_are_not_memories() {
        use super::ephemeral;
        for (text, what) in [
            ("Agent agent_01M5ST0WXECQ69JP9PZX76J1X4 created a personal skill named deliverable-premerge-check.", "an id"),
            ("In the docs project, deliverables/bye.txt was added on main by commit 85471b8 for ticket AGN-2.", "a commit hash"),
            ("deliverables/bye.txt contains two lines; it is 26 bytes, UTF-8 with no BOM.", "an exact file size"),
        ] {
            assert_eq!(ephemeral(text), Some(what), "{text}");
        }
        for text in [
            "In the docs project, deliverables must also satisfy the repo's pre-merge checks (the 'Owner:' first line).",
            "write_file refuses to overwrite an existing file until read_file has been called on it (error code read_first).",
            "Uploads over 50 MB fail on the staging server.",
            "Commit subjects in this project start with the ticket key, for example AGN-12: Add greeting.",
        ] {
            assert_eq!(ephemeral(text), None, "{text}");
        }
    }

    use super::*;

    #[test]
    fn json_in_prose() {
        let v = json_object("Sure!\n```json\n{\"facts\": [{\"text\": \"a {b} \\\"c\\\"\"}]}\n```")
            .unwrap();
        assert_eq!(v["facts"][0]["text"], "a {b} \"c\"");
        assert!(json_object("no json").is_none());
    }
}
