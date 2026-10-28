//! `find_code` on a real codebase with the real Jev API (ADR-128): ten behavioural questions about this repository, each
//! with the file that answers it. Measured the way jevgrep measures retrieval for coding agents: is the answering file
//! among the first files returned, is its code among the excerpts, how long a search takes, what it costs, and how much
//! the agent reads.
//!
//! Usage: `JEV_KEY=… cargo run -p agenium-eval --example find_code_eval [-- --root <repo>] [--only <n>]` (default: this
//! repository, every question; `FIND_CODE_SHOW=1` prints what each search returned).

use std::path::PathBuf;
use std::sync::Arc;

use agenium_orchestrator::judge::JudgeInput;
use anyhow::{anyhow, Result};

/// A question about this repository, the path fragment of the file that answers it, and a line its excerpt holds
/// (`data/find-code-cases.json`, shared with `find_code_ab`).
#[derive(serde::Deserialize)]
struct Case {
    question: String,
    file: String,
    line: String,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = args
        .iter()
        .position(|a| a == "--root")
        .and_then(|i| args.get(i + 1).map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let root = root.canonicalize()?;
    let key = std::env::var("JEV_KEY").map_err(|_| anyhow!("set JEV_KEY"))?;
    let tmp = tempfile::tempdir()?;
    let (app, _) = agenium_orchestrator::App::open(
        &tmp.path().join("root"),
        agenium_orchestrator::AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
        },
    )?;
    app.set_judge(&JudgeInput {
        base_url: "https://api.typesafe.ai".into(),
        model: "jev-1.14.0".into(),
        find_code: true,
        ..Default::default()
    })?;
    app.add_judge_key("eval", &key)?;
    println!("## find_code on {} (jev-1.14.0)\n", root.display());
    println!("| Question | answering file | rank | its code excerpted | files read by the judge | s | Jev input tokens | output tokens (≈ chars/4) |");
    println!("|---|---|---|---|---|---|---|---|");
    let (mut top3, mut excerpted, mut secs, mut jev, mut out_tokens) = (0, 0, 0.0, 0u64, 0usize);
    let mut cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/find-code-cases.json"),
    )?)?;
    // `--only <n>`: the n-th question alone (1-based), e.g. with FIND_CODE_SHOW=1 to see what it returned.
    if let Some(n) = args
        .iter()
        .position(|a| a == "--only")
        .and_then(|i| args.get(i + 1)?.parse::<usize>().ok())
    {
        cases = cases
            .into_iter()
            .skip(n.saturating_sub(1))
            .take(1)
            .collect();
    }
    for Case {
        question: q,
        file,
        line,
    } in &cases
    {
        let t = std::time::Instant::now();
        let out = app
            .find_code(&root, ".", q, None)
            .await
            .map_err(|e| anyhow!("{e}"))?;
        let s = t.elapsed().as_secs_f64();
        let rank = out
            .lines()
            .take_while(|l| !l.starts_with("---"))
            .filter_map(|l| {
                let (n, rest) = l.trim().split_once(". ")?;
                Some((n.parse::<usize>().ok()?, rest.to_owned()))
            })
            .find(|(_, rest)| rest.contains(file))
            .map(|(n, _)| n);
        let has_excerpt = out
            .split("\n--- ")
            .skip(1)
            .any(|b| b.lines().next().is_some_and(|h| h.contains(file)) && b.contains(line));
        let a = app.judge_activity(1);
        let tokens = a
            .first()
            .and_then(|x| x.usage.as_ref())
            .and_then(|u| u["input_tokens"].as_u64())
            .unwrap_or(0);
        let read = out
            .lines()
            .rev()
            .find_map(|l| {
                l.split_once(" read by the judge")
                    .map(|(a, _)| a.rsplit(' ').next().unwrap_or("?").to_owned())
            })
            .unwrap_or_default();
        top3 += usize::from(rank.is_some_and(|r| r <= 3));
        excerpted += usize::from(has_excerpt);
        secs += s;
        jev += tokens;
        out_tokens += out.len() / 4;
        println!(
            "| {q} | `{file}` | {} | {} | {read} | {s:.1} | {tokens} | {} |",
            rank.map_or("—".into(), |r| r.to_string()),
            if has_excerpt { "yes" } else { "no" },
            out.len() / 4
        );
        if std::env::var("FIND_CODE_SHOW").is_ok() {
            eprintln!("{out}");
        }
    }
    let n = cases.len();
    println!(
        "\nAnswering file in the top 3: {top3}/{n}; its code among the excerpts: {excerpted}/{n}. Mean {:.1} s and {} Jev \
         input tokens (≈ ${:.4}) per search; {} output tokens per search on average.",
        secs / n as f64,
        jev / n as u64,
        jev as f64 / n as f64 * 0.042 / 1e6,
        out_tokens / n
    );
    Ok(())
}
