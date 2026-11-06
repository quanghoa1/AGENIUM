//! Ranks items for queries through the production `retrieve::recall` (bge-small + BM25 + re-rank), for evaluations
//! done outside Rust (the trace-channel study, ADR-158 decision 30). Each store is built once in a temporary agent
//! database; each query is recalled in the stores it names.
//!
//! `cargo run --release -p agenium-memory --example recall_rank -- <stores.json> <queries.json> <models dir> <out.json> [depth]`
//! stores.json: `{"<store>": [{"id": "...", "text": "...", "source": "..."}]}`;
//! queries.json: `[{"id": "...", "text": "...", "stores": ["<store>", ...]}]`;
//! out.json: `{"<query id>": {"<store>": ["<item id>", ...]}}`, best first, `depth` items at most (default 60).

use std::collections::HashMap;

use agenium_memory::retrieve::{recall, RecallContext};
use agenium_store::{AgentStore, NewFact};
use serde_json::{json, Value};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let read =
        |p: &str| -> Value { serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap() };
    let (stores, queries) = (read(&a[1]), read(&a[2]));
    let depth: usize = a.get(5).and_then(|d| d.parse().ok()).unwrap_or(60);
    let e = agenium_memory::default_embedder(std::path::Path::new(&a[3]));
    assert!(
        e.model_id().contains("bge"),
        "the local model must be installed: {}",
        e.model_id()
    );
    let tmp = tempfile::tempdir().unwrap();
    let mut built: HashMap<String, (AgentStore, HashMap<i64, String>)> = HashMap::new();
    for (n, (key, items)) in stores.as_object().unwrap().iter().enumerate() {
        let dir = tmp.path().join(n.to_string());
        let store =
            AgentStore::open(&dir.join("m.db"), e.dim(), e.model_id(), &dir.join("pre")).unwrap();
        let mut ids = HashMap::new();
        let items = items.as_array().unwrap();
        for batch in items.chunks(32) {
            let texts: Vec<String> = batch
                .iter()
                .map(|x| x["text"].as_str().unwrap().to_owned())
                .collect();
            for (x, v) in batch.iter().zip(e.embed(&texts).unwrap()) {
                let f = NewFact::fact(
                    x["text"].as_str().unwrap(),
                    x["source"].as_str().unwrap_or(""),
                );
                ids.insert(
                    store.add(&f, Some(&v)).unwrap(),
                    x["id"].as_str().unwrap().to_owned(),
                );
            }
        }
        eprintln!("store {key}: {} items", items.len());
        built.insert(key.clone(), (store, ids));
    }
    let mut out = serde_json::Map::new();
    for q in queries.as_array().unwrap() {
        let mut per = serde_json::Map::new();
        for s in q["stores"].as_array().unwrap() {
            let s = s.as_str().unwrap();
            let Some((store, ids)) = built.get(s) else {
                continue;
            };
            let got: Vec<String> = recall(
                store,
                e.as_ref(),
                q["text"].as_str().unwrap(),
                &RecallContext::default(),
                depth,
            )
            .unwrap()
            .into_iter()
            .filter_map(|r| ids.get(&r.id).cloned())
            .collect();
            per.insert(s.to_owned(), json!(got));
        }
        out.insert(q["id"].as_str().unwrap().to_owned(), Value::Object(per));
    }
    std::fs::write(&a[4], serde_json::to_string(&Value::Object(out)).unwrap()).unwrap();
}
