//! The model's share of entity resolution (ADR-94): the pairs `agent/memory/examples/entity_resolution.rs --ask-out`
//! found in the ambiguous band (cosine 0.80..0.90, or a name the rules could not settle) go to a real model with the
//! pipeline's own question (`pipeline::SAME_RULES`, `pipeline::same_line`), in groups of ten as an episode would send
//! them. Reported: how often the model says "same" for pairs that are the same and for pairs that are not, and the
//! precision and recall of resolution as a whole (rules + model).
//!
//! `cargo run -p agenium-eval --example entity_ask -- --root <root> --provider <id> --model <model> --asks <file>`
//! Reads the provider and its key from that root.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use agenium_core::Effort;
use agenium_llm::types::{ChatRequest, Message, SystemBlock};
use agenium_memory::pipeline::{json_object, SAME_RULES};
use anyhow::{anyhow, Result};
use serde_json::Value;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(arg(&args, "--root").ok_or_else(|| anyhow!("--root required"))?);
    let provider_id = arg(&args, "--provider").ok_or_else(|| anyhow!("--provider required"))?;
    let model = arg(&args, "--model").ok_or_else(|| anyhow!("--model required"))?;
    let asks: Value = serde_json::from_str(&std::fs::read_to_string(
        arg(&args, "--asks").ok_or_else(|| anyhow!("--asks required"))?,
    )?)?;
    let keyring: Arc<dyn agenium_secrets::KeyringBackend> =
        Arc::new(agenium_secrets::OsKeyring::default());
    let (app, _) = agenium_orchestrator::App::open(
        &root,
        agenium_orchestrator::AppOptions {
            keyring,
            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
        },
    )?;
    let source = agenium_orchestrator::runtime::ConfiguredProviders::new(Arc::new(app));
    let provider = agenium_orchestrator::runtime::ProviderSource::provider(&source, &provider_id)?;

    let pairs = asks["asks"].as_array().cloned().unwrap_or_default();
    let (mut tokens_in, mut tokens_out, mut unreadable) = (0u64, 0u64, 0usize);
    // (labelled same, model said same) per mention kind.
    let mut by_kind: BTreeMap<String, [usize; 4]> = BTreeMap::new();
    let mut notes: Vec<String> = Vec::new();
    for chunk in pairs.chunks(10) {
        let mut q = String::new();
        for (n, p) in chunk.iter().enumerate() {
            let line = p["line"].as_str().unwrap_or_default();
            q.push_str(&format!(
                "{n}.{}",
                line.split_once('.').map(|x| x.1).unwrap_or(line)
            ));
        }
        let req = ChatRequest {
            model: model.clone(),
            system: vec![SystemBlock {
                text: SAME_RULES.into(),
                cache: false,
            }],
            messages: vec![Message::user_text(q)],
            tools: vec![],
            effort: Effort::LOW,
            max_tokens: 2000,
            cache_key: None,
            long_cache: false,
            tools_off: false,
        };
        let resp = provider.chat(&req, &|_| {}).await?;
        tokens_in += resp.usage.total_input();
        tokens_out += resp.usage.output;
        let said: Vec<bool> = json_object(&resp.message.text())
            .and_then(|v| v["same"].as_array().cloned())
            .map(|a| a.iter().map(|b| b.as_bool().unwrap_or(false)).collect())
            .unwrap_or_default();
        if said.len() != chunk.len() {
            unreadable += chunk.len();
            notes.push(format!(
                "unreadable answer ({} of {} verdicts): {}",
                said.len(),
                chunk.len(),
                resp.message.text().chars().take(300).collect::<String>()
            ));
        }
        for (i, p) in chunk.iter().enumerate() {
            let same = p["same"] == true;
            // As the pipeline does: a missing answer is "different".
            let yes = said.get(i).copied().unwrap_or(false);
            let k = by_kind
                .entry(p["kind"].as_str().unwrap_or("?").to_owned())
                .or_default();
            k[(same as usize) * 2 + yes as usize] += 1;
            if same != yes && i < said.len() {
                notes.push(format!(
                    "model said {}: {}",
                    if yes { "SAME" } else { "different" },
                    p["line"].as_str().unwrap_or_default().trim()
                ));
            }
        }
    }
    let total = |f: fn(&[usize; 4]) -> usize| by_kind.values().map(f).sum::<usize>();
    let (tn, fp, fneg, tp) = (
        total(|k| k[0]),
        total(|k| k[1]),
        total(|k| k[2]),
        total(|k| k[3]),
    );
    let auto_right = asks["auto_right"].as_u64().unwrap_or(0) as usize;
    let auto_wrong = asks["auto_wrong"].as_u64().unwrap_or(0) as usize;
    let positives = asks["positives"].as_u64().unwrap_or(1) as usize;
    println!(
        "model {provider_id}:{model} on {} ambiguous pairs ({} embedder, name-only vectors: {}); tokens in {tokens_in}, out {tokens_out}; unreadable answers {unreadable}",
        pairs.len(),
        asks["embedder"].as_str().unwrap_or("?"),
        asks["name_only"]
    );
    println!(
        "same pairs: model said same {tp} of {} | different pairs: model said same {fp} of {}",
        tp + fneg,
        tn + fp
    );
    println!(
        "resolution as a whole (rules + model): precision {:.4} ({} of {}), recall {:.3} ({} of {positives})",
        (auto_right + tp) as f64 / (auto_right + auto_wrong + tp + fp).max(1) as f64,
        auto_right + tp,
        auto_right + auto_wrong + tp + fp,
        (auto_right + tp) as f64 / positives as f64,
        auto_right + tp
    );
    println!();
    println!("| mention kind | different: model said different | different: model said SAME | same: model said different | same: model said same |");
    println!("|---|---|---|---|---|");
    for (k, v) in &by_kind {
        println!("| {k} | {} | {} | {} | {} |", v[0], v[1], v[2], v[3]);
    }
    for n in notes {
        println!("- {n}");
    }
    Ok(())
}
