//! Measures what Jev takes off the main agent (docs/jev-comparison.md): the right question for a decision-support model
//! is not "is Jev more accurate than an LLM" but "with Jev, how many tokens does the main agent avoid spending". Two
//! offloads are measured: memory_gate (the extraction call the gate skips on a routine ticket, priced live on the given
//! model) and skill_hint (the SKILL.md bytes the agent would read to choose, which Jev reads as short descriptions).
//!
//! Usage: `cargo run -p agenium-eval --example jev_offload -- --root <root> --provider <id> --model <model>
//! [--transcript <file>] [--skills <dir>]`. Reads the provider and key from that root (the extraction call costs tokens).

use std::path::PathBuf;
use std::sync::Arc;

use agenium_core::Effort;
use agenium_llm::types::{ChatRequest, Message, SystemBlock};
use agenium_memory::pipeline::EXTRACT_RULES;
use anyhow::{anyhow, Result};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

/// A representative routine-ticket work conversation, used when --transcript is not given.
const SAMPLE_TRANSCRIPT: &str = r###"Episode ticket:DEMO-14 (outcome: Closed: the work was accepted)
SIGNALS: none (no lessons)

## Ticket
Title: Fix typo in the README install section
Description: The word "dependancies" should be "dependencies" in README.md.
Acceptance: AC-1 the README spells "dependencies" correctly.

## Your work conversation
user: Do the work, then call submit_report.
assistant: I'll read the README and fix the typo.
tool read_file {"path":"README.md"} -> "## Install\nFirst install the dependancies with npm install..."
assistant: Found it on the install line. Editing.
tool edit_file {"path":"README.md","old":"dependancies","new":"dependencies"} -> ok
assistant: Running a quick grep to confirm no other occurrences.
tool shell {"command":"grep -n dependancies README.md"} -> (no matches)
assistant: Fixed. Submitting.
tool submit_report {"summary":"Corrected 'dependancies' to 'dependencies' in the README install section.","acceptance":[{"ac_id":"AC-1","status":"Met","evidence":"README.md install line"}]}
"###;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(arg(&args, "--root").ok_or_else(|| anyhow!("--root required"))?);
    let provider_id = arg(&args, "--provider").ok_or_else(|| anyhow!("--provider required"))?;
    let model = arg(&args, "--model").ok_or_else(|| anyhow!("--model required"))?;

    // --- memory_gate offload: price the extraction call the gate skips, on this model. ---
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

    let transcript = match arg(&args, "--transcript") {
        Some(p) => std::fs::read_to_string(p)?,
        None => SAMPLE_TRANSCRIPT.to_string(),
    };
    let req = ChatRequest {
        model: model.clone(),
        system: vec![SystemBlock {
            text: EXTRACT_RULES.into(),
            cache: false,
        }],
        messages: vec![Message::user_text(transcript.clone())],
        tools: vec![],
        effort: Effort::Low,
        max_tokens: 4000,
        cache_key: None,
        long_cache: false,
    };
    let t0 = std::time::Instant::now();
    let resp = provider
        .chat(&req, &|_| {})
        .await
        .map_err(|e| anyhow!(e.to_string()))?;
    let dt = t0.elapsed().as_secs_f64();
    let (ins, outs) = (resp.usage.total_input(), resp.usage.output);

    println!("=== memory_gate: main-agent tokens the gate SKIPS on a routine ticket ===");
    println!(
        "model={provider_id}:{model}  transcript_chars={}",
        transcript.len()
    );
    println!(
        "extraction call: input={ins} tokens, output={outs} tokens, total={}  ({dt:.2}s)",
        ins + outs
    );
    println!("With Jev gate on a confident 'nothing to remember', the main agent spends 0 of these; Jev pays its own budget.\n");

    // --- skill_hint offload: SKILL.md bytes the agent would read to choose, vs Jev reading descriptions. ---
    if let Some(dir) = arg(&args, "--skills") {
        let mut files: Vec<(String, usize)> = Vec::new();
        for e in walk(&PathBuf::from(&dir)) {
            if e.file_name().is_some_and(|n| n == "SKILL.md") {
                if let Ok(s) = std::fs::read_to_string(&e) {
                    files.push((e.display().to_string(), s.len()));
                }
            }
        }
        let total: usize = files.iter().map(|(_, n)| n).sum();
        println!(
            "=== skill_hint: main-agent tokens to choose among {} skills ===",
            files.len()
        );
        for (p, n) in &files {
            println!("  {}  ~{} tokens", p, n / 4);
        }
        println!(
            "reading ALL to choose: ~{} tokens on the main agent",
            total / 4
        );
        println!("Jev instead reads only name+description (~50 tokens each, ~{} total) and names the one to load;", files.len() * 50);
        println!("the main agent then reads just the chosen file. Tokens the main agent avoids grow with skill count and size.");
    } else {
        println!(
            "(pass --skills <dir> to also measure the skill_hint offload from real SKILL.md sizes)"
        );
    }
    Ok(())
}

fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}
