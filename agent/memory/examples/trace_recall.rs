//! Would a channel of raw traces help recall? (the Zero-Mem comparison, 2026-11-06). Measures on real episodes of the
//! QA runs (StudyDeck, Habits): the agents' live facts, and their tickets' raw traces (report, review, system, question
//! and answer comments) cut into chunks. Three conditions through the production `retrieve::recall`: facts only (today),
//! traces only, and both fused by reciprocal rank at the same budget of `K` items. Questions (made by a model from
//! sampled traces and facts, their answers copied from the passage) are scored by answer-hit@K (an item holds the
//! answer), source-hit@K (an item of the same ticket) and MRR of the first answer hit.
//!
//! `cargo run --release -p agenium-memory --example trace_recall -- <corpus.json> <questions.json> <models dir>`

use std::collections::HashMap;

use agenium_memory::retrieve::{recall, RecallContext};
use agenium_memory::Embedder;
use agenium_store::{AgentStore, NewFact};
use serde_json::Value;

const K: usize = 8;
const RRF_K: f64 = 60.0;
const TRACE_SLOTS: usize = 3;

struct Store {
    store: AgentStore,
    meta: HashMap<i64, (String, String)>, // id -> (source, text)
    _dir: tempfile::TempDir,
}

fn build(items: &[Value], e: &dyn Embedder) -> Store {
    let dir = tempfile::tempdir().unwrap();
    let store = AgentStore::open(
        &dir.path().join("m.db"),
        e.dim(),
        e.model_id(),
        &dir.path().join("pre"),
    )
    .unwrap();
    let mut meta = HashMap::new();
    for batch in items.chunks(32) {
        let texts: Vec<String> = batch
            .iter()
            .map(|x| x["text"].as_str().unwrap().to_owned())
            .collect();
        let vecs = e.embed(&texts).unwrap();
        for (x, v) in batch.iter().zip(vecs) {
            let src = x["source"].as_str().unwrap_or("").to_owned();
            let text = x["text"].as_str().unwrap().to_owned();
            let id = store
                .add(&NewFact::fact(text.clone(), src.clone()), Some(&v))
                .unwrap();
            meta.insert(id, (src, text));
        }
    }
    Store {
        store,
        meta,
        _dir: dir,
    }
}

/// The top `K` of a store for `q`: (source, text).
fn top(s: &Store, e: &dyn Embedder, q: &str) -> Vec<(String, String)> {
    recall(&s.store, e, q, &RecallContext::default(), K)
        .unwrap()
        .into_iter()
        .map(|r| s.meta.get(&r.id).cloned().unwrap_or_default())
        .collect()
}

/// Both lists fused by reciprocal rank, the first `K` kept: the same budget as one channel.
fn fuse(a: &[(String, String)], b: &[(String, String)]) -> Vec<(String, String)> {
    let mut score: Vec<(f64, (String, String))> = Vec::new();
    for list in [a, b] {
        for (rank, it) in list.iter().enumerate() {
            score.push((1.0 / (RRF_K + rank as f64 + 1.0), it.clone()));
        }
    }
    score.sort_by(|x, y| y.0.total_cmp(&x.0));
    score.into_iter().take(K).map(|(_, it)| it).collect()
}

#[derive(Default, Clone, Copy)]
struct Score {
    n: usize,
    answer: usize,
    source: usize,
    rr: f64,
}

impl Score {
    fn add(&mut self, got: &[(String, String)], answer: &str, source: &str) {
        let a = answer.to_lowercase();
        self.n += 1;
        if let Some(i) = got.iter().position(|(_, t)| t.to_lowercase().contains(&a)) {
            self.answer += 1;
            self.rr += 1.0 / (i as f64 + 1.0);
        }
        if got.iter().any(|(s, _)| s == source) {
            self.source += 1;
        }
    }
    fn line(&self) -> String {
        let n = self.n.max(1) as f64;
        format!(
            "answer-hit@{K} {:.2}  source-hit@{K} {:.2}  MRR {:.2}  (n={})",
            self.answer as f64 / n,
            self.source as f64 / n,
            self.rr / n,
            self.n
        )
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let corpus: Value = serde_json::from_str(&std::fs::read_to_string(&args[1]).unwrap()).unwrap();
    let questions: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(&args[2]).unwrap()).unwrap();
    let e = agenium_memory::default_embedder(std::path::Path::new(&args[3]));
    assert!(
        e.model_id().contains("bge"),
        "the local model must be installed, not the hashing stand-in: {}",
        e.model_id()
    );
    let mut scores: HashMap<(String, &str), Score> = HashMap::new();
    for (project, c) in corpus.as_object().unwrap() {
        let facts = build(c["facts"].as_array().unwrap(), e.as_ref());
        let traces = build(c["traces"].as_array().unwrap(), e.as_ref());
        for q in questions
            .iter()
            .filter(|q| q["project"] == project.as_str())
        {
            let (text, answer, source) = (
                q["question"].as_str().unwrap(),
                q["answer"].as_str().unwrap(),
                q["source"].as_str().unwrap(),
            );
            let origin = q["origin"].as_str().unwrap().to_owned();
            let f = top(&facts, e.as_ref(), text);
            let t = top(&traces, e.as_ref(), text);
            let both = fuse(&f, &t);
            // Facts keep the first places; traces take at most `TRACE_SLOTS` of the budget.
            let mut capped: Vec<(String, String)> =
                f.iter().take(K - TRACE_SLOTS).cloned().collect();
            capped.extend(t.iter().take(TRACE_SLOTS).cloned());
            for (cond, got) in [
                ("facts", &f),
                ("traces", &t),
                ("fused", &both),
                ("capped", &capped),
            ] {
                scores
                    .entry((origin.clone(), cond))
                    .or_default()
                    .add(got, answer, source);
                scores
                    .entry(("all".into(), cond))
                    .or_default()
                    .add(got, answer, source);
            }
        }
    }
    for origin in ["trace", "fact", "all"] {
        println!("{origin} questions:");
        for cond in ["facts", "traces", "fused", "capped"] {
            if let Some(s) = scores.get(&(origin.to_owned(), cond)) {
                println!("  {cond:<7} {}", s.line());
            }
        }
    }
}
