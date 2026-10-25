//! Baseline half of the Jev A/B (ADR-85, docs/jev-comparison.md): runs the same labelled judge dataset through a
//! configured chat model and reports accuracy, Brier calibration, latency and tokens, so it can be set beside the Jev
//! numbers. Usage:
//!   cargo run -p agenium-eval --example jev_baseline -- --root <root> --provider <id> --model <model> --data <dataset.json>
//! Reads the provider and its key from that root (costs tokens). Not a test; run by hand.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use agenium_core::Effort;
use agenium_llm::types::{ChatRequest, Message, SystemBlock};
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
    let data_path = arg(&args, "--data").ok_or_else(|| anyhow!("--data required"))?;

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

    let data: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(&data_path)?)?;
    let system = "You are a strict evaluator. Read the statement and answer the question using ONLY the statement. \
        Reply with ONE number between 0 and 1 and nothing else: the probability that the answer to the question is YES.";

    let (mut correct, mut brier, mut n) = (0usize, 0.0f64, 0usize);
    let (mut tot_in, mut tot_out, mut tot_dt) = (0u64, 0u64, 0.0f64);
    for it in &data {
        let state = it["state"].as_str().unwrap_or_default();
        let q = it["q"].as_str().unwrap_or_default();
        let label = it["label"].as_bool().unwrap_or(false);
        let req = ChatRequest {
            model: model.clone(),
            system: vec![SystemBlock {
                text: system.into(),
                cache: false,
            }],
            messages: vec![Message::user_text(format!(
                "STATEMENT:\n{state}\n\nQUESTION: {q}"
            ))],
            tools: vec![],
            effort: Effort::Low,
            max_tokens: 800,
            cache_key: None,
            long_cache: false,
        };
        let t0 = Instant::now();
        let resp = match provider.chat(&req, &|_| {}).await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{}: ERR {e}", it["id"].as_str().unwrap_or("?"));
                continue;
            }
        };
        let dt = t0.elapsed().as_secs_f64();
        let text = resp.message.text();
        let p = text
            .split(|c: char| !(c.is_ascii_digit() || c == '.'))
            .find(|s| !s.is_empty())
            .and_then(|s| s.parse::<f64>().ok())
            .map(|x| x.clamp(0.0, 1.0));
        let pred = p.map(|x| x >= 0.5).unwrap_or(false);
        let ok = pred == label;
        correct += ok as usize;
        if let Some(x) = p {
            brier += (x - if label { 1.0 } else { 0.0 }).powi(2);
        }
        tot_in += resp.usage.total_input();
        tot_out += resp.usage.output;
        tot_dt += dt;
        n += 1;
        println!(
            "{:5}  label={:5}  p={:?}  pred={}  {}  {:.2}s  in={} out={}  raw={:?}",
            it["id"].as_str().unwrap_or("?"),
            label,
            p,
            pred,
            if ok { "OK" } else { "XX" },
            dt,
            resp.usage.total_input(),
            resp.usage.output,
            text.chars().take(20).collect::<String>()
        );
    }
    let n = n.max(1);
    println!(
        "\n=== BASELINE {provider_id}:{model} ===\nn={n}  accuracy={correct}/{n}={:.3}  Brier={:.4}  avg_latency={:.2}s  avg_in={}  avg_out={}",
        correct as f64 / n as f64, brier / n as f64, tot_dt / n as f64, tot_in / n as u64, tot_out / n as u64
    );
    Ok(())
}
