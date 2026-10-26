//! Probes a provider's prompt cache with controlled requests (ADR-113): which changes keep the cached prefix and which
//! lose it. Each case sends a warm-up request, waits, then the variant, and prints how many input tokens the variant
//! read from the cache. Costs a few small requests.
//!
//! cargo run -p agenium-eval --example cache_probe -- --root <root> --provider <id> --model <model>
//! cargo run -p agenium-eval --example cache_probe -- --base-url <url|default> --key-env <VAR> [--kind anthropic] --provider <id> --model <model>

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agenium_core::Effort;
use agenium_llm::*;
use anyhow::{anyhow, Result};
use serde_json::json;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn tool(name: &str, words: usize) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: format!(
            "{name}: {}",
            "Reads part of the project and reports what it found. ".repeat(words)
        ),
        parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
    }
}

fn req(model: &str, system: &[&str], tools: Vec<ToolSpec>, user: &str, key: &str) -> ChatRequest {
    ChatRequest {
        model: model.into(),
        system: system
            .iter()
            .map(|t| SystemBlock {
                text: (*t).into(),
                cache: true,
            })
            .collect(),
        messages: vec![Message::user_text(user)],
        tools,
        effort: Effort::LOW,
        max_tokens: 64,
        cache_key: Some(key.into()),
        long_cache: false,
        tools_off: false,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let provider_id = arg(&args, "--provider").ok_or_else(|| anyhow!("--provider"))?;
    let model = arg(&args, "--model").ok_or_else(|| anyhow!("--model"))?;
    // Either a root's configured provider (key in its vault), or a provider built from `--base-url` and the key in
    // the environment variable `--key-env` (a test key that must not be stored).
    let p: Arc<dyn Provider> = match arg(&args, "--base-url") {
        Some(base) => {
            let var =
                arg(&args, "--key-env").ok_or_else(|| anyhow!("--key-env with --base-url"))?;
            let key = std::env::var(&var).map_err(|_| anyhow!("{var} is not set"))?;
            build_provider(&ProviderSpec {
                id: provider_id.clone(),
                kind: arg(&args, "--kind").unwrap_or_else(|| "openai_compat".into()),
                base_url: (base != "default").then_some(base),
                api_key: Some(key),
                max_concurrency: 1,
            })?
        }
        None => {
            let root =
                PathBuf::from(arg(&args, "--root").ok_or_else(|| anyhow!("--root or --base-url"))?);
            let (app, _) = agenium_orchestrator::App::open(
                &root,
                agenium_orchestrator::AppOptions {
                    keyring: Arc::new(agenium_secrets::OsKeyring::default()),
                    checks: Arc::new(agenium_orchestrator::NoCheckFactory),
                },
            )?;
            let source = agenium_orchestrator::runtime::ConfiguredProviders::new(Arc::new(app));
            agenium_orchestrator::runtime::ProviderSource::provider(&source, &provider_id)?
        }
    };
    // A unique stem per probe so earlier probes' cache does not answer.
    let stem = format!("Probe {}. ", agenium_core::now_iso());
    let rules = format!(
        "{stem}{}",
        "Rule: verify your work before reporting it, with evidence. ".repeat(220)
    );
    let role = "Role: you are a careful backend engineer. ".repeat(120);
    let tools_a = vec![tool("alpha", 60), tool("beta", 60), tool("gamma", 60)];
    let tools_b = vec![tool("alpha", 60), tool("delta", 60), tool("gamma", 60)];
    let send = |r: ChatRequest| {
        let p = p.clone();
        async move {
            let out = p.chat(&r, &|_| {}).await.map_err(|e| anyhow!("{e}"))?;
            Ok::<_, anyhow::Error>((out.usage.total_input(), out.usage.cache_read))
        }
    };
    let cases: Vec<(&str, ChatRequest)> = vec![
        (
            "same prefix, new user message",
            req(
                &model,
                &[&rules, &role],
                tools_a.clone(),
                "Say ok. (2)",
                "k1",
            ),
        ),
        (
            "a block appended at the end of the system",
            req(
                &model,
                &[&rules, &role, "Worktree: C:/wt/AGN-7"],
                tools_a.clone(),
                "Say ok. (3)",
                "k1",
            ),
        ),
        (
            "one tool replaced (tools differ)",
            req(
                &model,
                &[&rules, &role],
                tools_b.clone(),
                "Say ok. (4)",
                "k1",
            ),
        ),
        (
            "no tools at all",
            req(&model, &[&rules, &role], vec![], "Say ok. (5)", "k1"),
        ),
        (
            "same prefix, another session key",
            req(
                &model,
                &[&rules, &role],
                tools_a.clone(),
                "Say ok. (6)",
                "k2-other",
            ),
        ),
        (
            "the second system block changed",
            req(
                &model,
                &[&rules, "Role: you are a frontend engineer."],
                tools_a.clone(),
                "Say ok. (7)",
                "k1",
            ),
        ),
    ];
    let (inp, cached) = send(req(
        &model,
        &[&rules, &role],
        tools_a.clone(),
        "Say ok. (1)",
        "k1",
    ))
    .await?;
    println!("warm-up: {cached} of {inp} input tokens cached");
    // A growing conversation (the messages part of the cache): the same history with one more turn, sent with the
    // tools callable and then with the tools listed but not callable (a notes request, ADR-113).
    let long = format!(
        "{stem}Context: {}",
        "The service keeps its records in a ledger. ".repeat(300)
    );
    let conv = |turns: &[(&str, &str)], tools_off: bool| {
        let mut r = req(&model, &[&rules, &role], tools_a.clone(), &long, "k1");
        for (a, u) in turns {
            r.messages.push(Message {
                role: Role::Assistant,
                parts: vec![Part::Text { text: (*a).into() }],
            });
            r.messages.push(Message::user_text(*u));
        }
        r.tools_off = tools_off;
        r
    };
    let (i1, c1) = send(conv(&[], false)).await?;
    println!("conversation, turn 1: {c1} of {i1} input tokens cached");
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (i2, c2) = send(conv(&[("Noted.", "Say ok.")], false)).await?;
    println!(
        "conversation, turn 2 (tools callable): {c2} of {i2} cached ({:.0} %)",
        100.0 * c2 as f64 / i2.max(1) as f64
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (i3, c3) = send(conv(
        &[("Noted.", "Say ok."), ("ok", "Write one line of notes.")],
        true,
    ))
    .await?;
    println!(
        "conversation, turn 3 (tools listed, not callable): {c3} of {i3} cached ({:.0} %)",
        100.0 * c3 as f64 / i3.max(1) as f64
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    let (i4, c4) = send(conv(
        &[
            ("Noted.", "Say ok."),
            ("ok", "Write one line of notes, no tools."),
        ],
        false,
    ))
    .await?;
    println!(
        "conversation, turn 3' (tools callable, told not to call): {c4} of {i4} cached ({:.0} %)",
        100.0 * c4 as f64 / i4.max(1) as f64
    );
    for (name, r) in cases {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let (inp, cached) = send(r).await?;
        println!(
            "{name}: {cached} of {inp} input tokens cached ({:.0} %)",
            100.0 * cached as f64 / inp.max(1) as f64
        );
    }
    Ok(())
}
