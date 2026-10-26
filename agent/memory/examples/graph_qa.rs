//! Multi-hop QA on a labelled set (ADR-51 §13 item 4): does the graph channel find the memory that answers a question
//! the text channels alone cannot place? `data/graph_qa.json` (from `data/make_datasets.py`) holds a synthetic
//! operations world as ideal extractions (214 facts, 36 episodes) and 112 questions labelled with the fact that answers
//! each: single-hop (the component is named), two-hop (only the service that uses the component is named), two-hop
//! through a team (team -> service -> component) and scope traps (a product is named; the same symptom's fix for
//! another vendor must not be shown).
//!
//! Each episode goes through the real `pipeline::learn` (a stub model returns the episode's extraction, adds every new
//! fact and answers "different" for ambiguous entities), so resolution, linking and edges are the production ones. The
//! same facts are learned a second time with entities and relations removed: that store has no graph, so recall is the
//! plain vector + BM25 hybrid with the scope filter. Every question runs `retrieve::recall` (k = 8) on both.
//! Measured: recall@8 and MRR of the labelled fact, and scope leaks (a forbidden fact in the top 8).
//!
//! `cargo run --release -p agenium-memory --example graph_qa -- [<models dir with bge-small-en-v1.5>] [--misses]`

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use agenium_llm::mock::ScriptedProvider;
use agenium_llm::{ChatResponse, Message, RetryPolicy, StopReason, Usage};
use agenium_memory::pipeline::{learn, Episode, PipelineModel};
use agenium_memory::retrieve::{recall, RecallContext};
use agenium_memory::{default_embedder, Embedder, HashEmbedder};
use agenium_store::AgentStore;
use serde_json::{json, Value};

const K: usize = 8;

/// A stand-in for the memory model: the current episode's extraction, ADD for every new fact, "different" for every
/// ambiguous entity pair.
fn responder(current: Arc<Mutex<String>>) -> agenium_llm::mock::Responder {
    Box::new(move |req| {
        let prompt = req.messages.last().map(Message::text).unwrap_or_default();
        let text = if prompt.starts_with("Episode ") {
            current.lock().unwrap().clone()
        } else if prompt.starts_with("NEW ") {
            let n = prompt.lines().filter(|l| l.starts_with("NEW ")).count();
            json!({"ops": (0..n).map(|i| json!({"op": "ADD", "fact": i})).collect::<Vec<_>>()})
                .to_string()
        } else {
            let n = prompt
                .lines()
                .filter(|l| {
                    l.split_once(". ")
                        .is_some_and(|(a, _)| a.parse::<usize>().is_ok())
                })
                .count();
            json!({"same": vec![false; n]}).to_string()
        };
        Ok(ChatResponse {
            message: Message::assistant_text(text),
            stop: StopReason::EndTurn,
            usage: Usage::default(),
            model: "stub".into(),
            dropped_thinking: 0,
        })
    })
}

async fn build(
    d: &Value,
    e: &Arc<dyn Embedder>,
    graph: bool,
    dir: &Path,
) -> (AgentStore, HashMap<i64, String>) {
    let store =
        AgentStore::open(&dir.join("qa.db"), e.dim(), e.model_id(), &dir.join("b")).unwrap();
    let current = Arc::new(Mutex::new(String::new()));
    let p = ScriptedProvider::new("stub");
    let model = PipelineModel {
        model: "stub".into(),
        effort: agenium_core::Effort::LOW,
        max_tokens: 2000,
        retry: RetryPolicy {
            max_attempts: 1,
            ..RetryPolicy::default()
        },
    };
    let project = d["project"].as_str().unwrap();
    let mut key_of_text: HashMap<String, String> = HashMap::new();
    for ep in d["episodes"].as_array().unwrap() {
        let facts: Vec<Value> = ep["facts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                key_of_text.insert(
                    f["text"].as_str().unwrap().to_owned(),
                    f["key"].as_str().unwrap().to_owned(),
                );
                let mut f = f.clone();
                if !graph {
                    f["entities"] = json!([]);
                    f["relations"] = json!([]);
                }
                f
            })
            .collect();
        *current.lock().unwrap() = json!({ "facts": facts }).to_string();
        for _ in 0..3 {
            p.push(responder(current.clone()));
        }
        let r = learn(
            &p,
            &model,
            &store,
            e.as_ref(),
            &Episode {
                source: ep["source"].as_str().unwrap().into(),
                project: Some(format!("p-{project}")),
                project_name: Some(project.into()),
                outcome: "Closed: merged".into(),
                transcript: "...".into(),
                signals: vec!["accepted after 1 reopen(s)".into()],
                ..Default::default()
            },
            &|t| Some(t.to_owned()),
        )
        .await
        .unwrap();
        assert!(r.rejected.is_empty(), "{:?}", r.rejected);
    }
    // Fact id -> dataset key.
    let mut keys = HashMap::new();
    for f in store.live(10_000).unwrap() {
        if let Some(k) = key_of_text.get(&f.text) {
            keys.insert(f.id, k.clone());
        }
    }
    (store, keys)
}

#[derive(Default, Clone)]
struct Score {
    n: usize,
    hits: usize,
    rr: f64,
    leaks: usize,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let misses = args.iter().any(|a| a == "--misses");
    let models = args.iter().find(|a| !a.starts_with("--"));
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/data/graph_qa.json");
    let d: Value =
        serde_json::from_str(&std::fs::read_to_string(&data).expect("graph_qa.json")).unwrap();
    let mut embedders: Vec<Arc<dyn Embedder>> = vec![Arc::new(HashEmbedder::new(384))];
    if let Some(m) = models {
        let e = default_embedder(Path::new(m));
        if !e.model_id().starts_with("hash") {
            embedders.push(e);
        }
    }
    let project = d["project"].as_str().unwrap();
    let ctx = RecallContext {
        project: Some(format!("p-{project}")),
        scope: json!({ "project": project }),
    };
    let types = ["single-hop", "two-hop", "two-hop-team", "scope-trap"];
    println!("| embedder | graph | question type | n | recall@{K} | MRR | scope leaks |");
    println!("|---|---|---|---|---|---|---|");
    let mut miss_lines = Vec::new();
    for e in &embedders {
        for graph in [false, true] {
            let tmp = tempfile::tempdir().unwrap();
            let (store, keys) = build(&d, e, graph, tmp.path()).await;
            assert_eq!(
                keys.len() as u64,
                store.count_live().unwrap(),
                "every fact stored once"
            );
            let mut by: HashMap<&str, Score> = HashMap::new();
            for q in d["questions"].as_array().unwrap() {
                let typ = q["type"].as_str().unwrap();
                let gold: Vec<&str> = q["gold"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                let forbidden: Vec<&str> = q["forbidden"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                let got = recall(&store, e.as_ref(), q["text"].as_str().unwrap(), &ctx, K).unwrap();
                let got: Vec<&str> = got
                    .iter()
                    .map(|r| keys.get(&r.id).map(String::as_str).unwrap_or("?"))
                    .collect();
                let rank = got.iter().position(|k| gold.contains(k));
                let leak = got.iter().any(|k| forbidden.contains(k));
                for t in [typ, "all"] {
                    let s = by.entry(t).or_default();
                    s.n += 1;
                    if let Some(r) = rank {
                        s.hits += 1;
                        s.rr += 1.0 / (r + 1) as f64;
                    }
                    s.leaks += leak as usize;
                }
                if misses && graph && (rank.is_none() || leak) {
                    miss_lines.push(format!(
                        "[{}] {} {}: {}\n    gold {:?}, got {:?}",
                        e.model_id(),
                        q["id"].as_str().unwrap(),
                        typ,
                        q["text"].as_str().unwrap(),
                        gold,
                        got
                    ));
                }
            }
            for t in types.iter().chain(["all"].iter()) {
                let s = by.get(t).cloned().unwrap_or_default();
                println!(
                    "| {} | {} | {t} | {} | {:.2} | {:.2} | {} |",
                    e.model_id(),
                    if graph { "on" } else { "off" },
                    s.n,
                    s.hits as f64 / s.n.max(1) as f64,
                    s.rr / s.n.max(1) as f64,
                    s.leaks
                );
            }
        }
    }
    for l in miss_lines {
        println!("{l}");
    }
}
