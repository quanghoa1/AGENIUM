//! Entity-resolution precision on a labelled set (ADR-51 §13 item 3): `data/entity_pairs.json` (from
//! `data/make_datasets.py`) holds 115 entities as stored in memory and ~2 150 mentions labelled with the entity each one
//! names, or none. Every mention goes through the production path of the memory pipeline: embed `entity_text(name)`,
//! `AgentStore::entity_candidates` (exact key, alias, FTS, vector), `resolve::decide`. Reported: auto-merge precision
//! (target >= 0.98), how many mentions would go to the model (the 0.80..0.90 band), what each kind of mention gets, and
//! the similarity of the best vector candidate per band for same vs different pairs.
//!
//! `cargo run --release -p agenium-memory --example entity_resolution -- [<models dir with bge-small-en-v1.5>]
//! [--describe] [--ask-out <file>]`
//! Without a models dir (or without the model in it) the hashing embedder is used. `--describe` stores entity vectors of
//! "name. description" as before ADR-94 (production embeds the name alone). `--ask-out` writes the pairs of the ambiguous band, with their labels, for
//! `quality/eval/examples/entity_ask.rs` (which puts them to a real model with the pipeline's question).

use std::collections::BTreeMap;
use std::path::Path;

use agenium_memory::default_embedder;
use agenium_memory::resolve::{decide, entity_text, Decision, ASK_SAME, AUTO_SAME};
use agenium_store::graph::NewEntity;
use agenium_store::AgentStore;
use serde_json::Value;

#[derive(Default)]
struct Tally {
    n: usize,
    same_right: usize,
    same_wrong: usize,
    ask_right: usize,
    ask_wrong: usize,
    new_right: usize,
    new_wrong: usize,
}

impl Tally {
    fn add(&mut self, gold: Option<i64>, d: &Decision) {
        self.n += 1;
        match (d, gold) {
            (Decision::Same(id, _), Some(g)) if *id == g => self.same_right += 1,
            (Decision::Same(..), _) => self.same_wrong += 1,
            (Decision::Ask(id, _), Some(g)) if *id == g => self.ask_right += 1,
            (Decision::Ask(..), _) => self.ask_wrong += 1,
            (Decision::New, None) => self.new_right += 1,
            (Decision::New, Some(_)) => self.new_wrong += 1,
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The design before ADR-94: entity vectors of "name. description".
    let describe = args.iter().any(|a| a == "--describe");
    let ask_out = args
        .iter()
        .position(|a| a == "--ask-out")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let models = args
        .iter()
        .enumerate()
        .find(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--ask-out"))
        .map(|(_, a)| a.clone());
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/data/entity_pairs.json");
    let d: Value =
        serde_json::from_str(&std::fs::read_to_string(&data).expect("entity_pairs.json")).unwrap();
    let e = default_embedder(Path::new(models.as_deref().unwrap_or("-")));
    let tmp = tempfile::tempdir().unwrap();
    let store = AgentStore::open(
        &tmp.path().join("er.db"),
        e.dim(),
        e.model_id(),
        &tmp.path().join("b"),
    )
    .unwrap();

    // The catalogue as memory holds it: name, identity scope, description, aliases, vector of name + description.
    let mut ids: BTreeMap<String, i64> = BTreeMap::new();
    for x in d["entities"].as_array().unwrap() {
        let s = |k: &str| x[k].as_str().unwrap_or_default().to_owned();
        let ne = NewEntity {
            kind: s("type"),
            name: s("name"),
            scope: x["scope"].clone(),
            description: Some(s("description")),
            aliases: x["aliases"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|a| a.as_str().map(str::to_owned))
                .collect(),
            confidence: 0.6,
            evidence: None,
        };
        let text = if describe {
            format!("{}. {}", ne.name, ne.description.as_deref().unwrap_or(""))
        } else {
            entity_text(&ne.name)
        };
        let v = e.embed(&[text]).unwrap().pop();
        let (id, created) = store.upsert_entity(&ne, v.as_deref()).unwrap();
        assert!(created, "duplicate entity {}", ne.name);
        ids.insert(s("id"), id);
    }

    let mut by_kind: BTreeMap<String, Tally> = BTreeMap::new();
    let mut all = Tally::default();
    // Best vector similarity per pair, split by label.
    let mut sims: Vec<(f64, bool)> = Vec::new();
    let mut wrong: Vec<String> = Vec::new();
    let mut asks: Vec<Value> = Vec::new();
    for p in d["pairs"].as_array().unwrap() {
        let mention = NewEntity {
            kind: p["type"].as_str().unwrap().to_owned(),
            name: p["mention"].as_str().unwrap().to_owned(),
            scope: p["scope"].clone(),
            description: None,
            aliases: vec![],
            confidence: 0.6,
            evidence: None,
        };
        let gold = p["gold"].as_str().map(|g| ids[g]);
        let v = e.embed(&[entity_text(&mention.name)]).unwrap().pop();
        let cands = store
            .entity_candidates(&mention.kind, &mention.name, v.as_deref(), 5)
            .unwrap();
        let dec = decide(&mention, &cands);
        if let Decision::Ask(id, sim) = dec {
            let e = store.entity(id).unwrap();
            asks.push(serde_json::json!({
                "pair": p["id"], "kind": p["kind"], "same": gold == Some(id),
                "line": agenium_memory::pipeline::same_line(0, &mention, &e, sim),
            }));
        }
        if let Some(best) = cands
            .iter()
            .filter(|c| c.via == "vector")
            .map(|c| c.similarity)
            .reduce(f64::max)
        {
            sims.push((best, gold.is_some()));
        }
        let kind = p["kind"].as_str().unwrap().to_owned();
        by_kind.entry(kind.clone()).or_default().add(gold, &dec);
        all.add(gold, &dec);
        let bad = match (&dec, gold) {
            (Decision::Same(id, _), g) => g != Some(*id),
            _ => false,
        };
        if bad {
            let to = cands
                .iter()
                .find(|c| matches!(dec, Decision::Same(id, _) if id == c.entity.id))
                .map(|c| format!("{} via {} {:.2}", c.entity.name, c.via, c.similarity))
                .unwrap_or_default();
            wrong.push(format!(
                "{kind}: \"{}\" ({}) merged into {to}",
                mention.name, mention.kind
            ));
        }
    }

    let same_total = all.same_right + all.same_wrong;
    let positives: usize = d["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| !p["gold"].is_null())
        .count();
    println!(
        "embedder: {} | {} entities, {} mentions ({positives} same, {} different) | bands: auto >= {AUTO_SAME} + string-similar, ask >= {ASK_SAME}",
        e.model_id(),
        ids.len(),
        all.n,
        all.n - positives
    );
    println!(
        "auto-merge precision {:.4} ({} of {same_total}), auto-merge recall {:.3} ({} of {positives}), sent to the model {} ({:.1} %), false merges {}",
        all.same_right as f64 / same_total.max(1) as f64,
        all.same_right,
        all.same_right as f64 / positives.max(1) as f64,
        all.same_right,
        all.ask_right + all.ask_wrong,
        100.0 * (all.ask_right + all.ask_wrong) as f64 / all.n as f64,
        all.same_wrong
    );
    println!();
    println!("| mention kind | n | same: right | same: WRONG | ask: right entity | ask: other/none | new: right | new: missed |");
    println!("|---|---|---|---|---|---|---|---|");
    for (k, t) in &by_kind {
        println!(
            "| {k} | {} | {} | {} | {} | {} | {} | {} |",
            t.n, t.same_right, t.same_wrong, t.ask_right, t.ask_wrong, t.new_right, t.new_wrong
        );
    }
    println!();
    println!("Best vector candidate similarity (same type), by label:");
    println!("| band | same | different | share same |");
    println!("|---|---|---|---|");
    let bands = [0.0, 0.5, 0.6, 0.7, 0.8, 0.85, 0.9, 0.95, 1.01];
    for w in bands.windows(2) {
        let (lo, hi) = (w[0], w[1]);
        let s = sims
            .iter()
            .filter(|(x, p)| *p && *x >= lo && *x < hi)
            .count();
        let o = sims
            .iter()
            .filter(|(x, p)| !*p && *x >= lo && *x < hi)
            .count();
        println!(
            "| {lo:.2}..{:.2} | {s} | {o} | {} |",
            hi.min(1.0),
            if s + o == 0 {
                "-".into()
            } else {
                format!("{:.2}", s as f64 / (s + o) as f64)
            }
        );
    }
    if let Some(path) = ask_out {
        let doc = serde_json::json!({
            "embedder": e.model_id(), "name_only": !describe, "mentions": all.n, "positives": positives,
            "auto_right": all.same_right, "auto_wrong": all.same_wrong, "asks": asks,
        });
        std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap()).unwrap();
        println!("ambiguous pairs written to {path}");
    }
    if !wrong.is_empty() {
        println!();
        println!("False merges:");
        for w in &wrong {
            println!("- {w}");
        }
    }
}
