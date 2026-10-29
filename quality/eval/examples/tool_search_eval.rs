//! `tool_search` on 123 tools shaped like real MCP servers' (ADR-139): 58 requests worded unlike the tool descriptions
//! ("give the customer their money back" → `stripe_create_refund`), each with the tool that does it
//! (`agent/mcp/data/many-tools-queries.json`). Ranked by words alone (BM25), by words and meaning (the local
//! `bge-small-en-v1.5`), and with the judge (Jev) reading the best candidates when `JEV_KEY` is set. Reports whether the
//! right tool is first, in the first 3 and in the first 5 (a search returns 5), the mean reciprocal rank, and time.
//!
//! Usage: `cargo build -p agenium-mcp --bin agenium-mcp-many-tools` first, then
//! `[JEV_KEY=…] cargo run -p agenium-eval --example tool_search_eval -- --models <root>/models`
//! (`--models`: a folder holding `bge-small-en-v1.5`, linked into the scratch root; without it, meaning is the hashing
//! stand-in). `--opaque`: the tools named tool_000…, their descriptions kept. `TOOL_SEARCH_SHOW=1` prints the misses.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};

#[derive(serde::Deserialize)]
struct Case {
    query: String,
    expect: String,
}

fn link_models(from: &Path, root: &Path) -> Result<()> {
    let model = from.join("bge-small-en-v1.5");
    let to = root.join("models").join("bge-small-en-v1.5");
    std::fs::create_dir_all(&to)?;
    for e in std::fs::read_dir(&model)? {
        let e = e?;
        if e.file_type()?.is_file() {
            let dest = to.join(e.file_name());
            if std::fs::hard_link(e.path(), &dest).is_err() {
                std::fs::copy(e.path(), &dest)?;
            }
        }
    }
    Ok(())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let server = arg("--server").map(PathBuf::from).unwrap_or_else(|| {
        repo.join("target/debug").join(format!(
            "agenium-mcp-many-tools{}",
            std::env::consts::EXE_SUFFIX
        ))
    });
    if !server.exists() {
        return Err(anyhow!(
            "{} is missing: cargo build -p agenium-mcp --bin agenium-mcp-many-tools",
            server.display()
        ));
    }
    let tmp = tempfile::tempdir()?;
    let root = tmp.path().join("root");
    let (app, _) = agenium_orchestrator::App::open(
        &root,
        agenium_orchestrator::AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
        },
    )?;
    if let Some(m) = arg("--models") {
        link_models(Path::new(&m), &root)?;
    }
    let app = Arc::new(app);
    println!("embedder: {}", app.embedder().model_id());
    let hub = agenium_mcp::McpHub::new(Arc::new(|_: &str| None), tmp.path().join("mcp"));
    // `--opaque`: the tools named tool_000… (descriptions kept), so only the descriptions say what a tool does.
    let opaque = args.iter().any(|a| a == "--opaque");
    let cfg = agenium_config::McpServerConfig {
        id: "many".into(),
        command: Some(server.display().to_string()),
        args: if opaque {
            vec!["--opaque".into()]
        } else {
            vec![]
        },
        env: Default::default(),
        url: None,
        headers: Default::default(),
        timeout_secs: 30,
        oauth: None,
    };
    let b = agenium_mcp::Binding {
        server: "many".into(),
        credentials: Default::default(),
        keys: Default::default(),
        tools: None,
    };
    let m = hub.tools_for("eval", &[b], &[cfg]).await;
    if !m.problems.is_empty() {
        return Err(anyhow!("{:?}", m.problems));
    }
    let catalog = Arc::new(m.catalog(Arc::new(|_| true)));
    let mut cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(
        repo.join("agent/mcp/data/many-tools-queries.json"),
    )?)?;
    if opaque {
        let names: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(
            repo.join("agent/mcp/data/many-tools.json"),
        )?)?;
        for c in &mut cases {
            let i = names.iter().position(|t| t["name"] == c.expect.as_str());
            c.expect = format!(
                "tool_{:03}",
                i.ok_or_else(|| anyhow!("no tool {}", c.expect))?
            );
        }
    }
    let show = std::env::var("TOOL_SEARCH_SHOW").is_ok();
    // (label, rank by meaning too, the judge's `[tool_search] judge` mode)
    let mut arms = vec![
        ("words (BM25)", false, "off"),
        ("words + meaning", true, "off"),
    ];
    let key = std::env::var("JEV_KEY").ok();
    if let Some(key) = &key {
        app.set_judge(&agenium_orchestrator::judge::JudgeInput {
            base_url: "https://api.typesafe.ai".into(),
            model: "jev-1.14.0".into(),
            ..Default::default()
        })?;
        app.add_judge_key("eval", key)?;
        arms.push(("+ judge on every search", true, "always"));
        arms.push(("+ judge when unsure (default)", true, "unsure"));
    }
    println!(
        "\n| Ranking | first | in 3 | in 5 | MRR | ms / search | judge reads | judge tokens / search |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    for (label, meaning, judge) in arms {
        app.update_config(|c| c.tool_search.judge = judge.into())?;
        // The first search embeds the catalogue; that is paid once per process, not per search.
        let _ = app
            .find_mcp_tools(catalog.clone(), "warm up", 5, meaning)
            .await;
        let before = app.judge_activity(1000).len();
        let (mut first, mut in3, mut in5, mut rr, mut ms) = (0, 0, 0, 0.0, 0u128);
        for c in &cases {
            let t = std::time::Instant::now();
            let got = app
                .find_mcp_tools(catalog.clone(), &c.query, 5, meaning)
                .await;
            ms += t.elapsed().as_millis();
            let rank = got.iter().position(|g| *g == c.expect);
            if rank == Some(0) {
                first += 1;
            }
            if rank.is_some_and(|r| r < 3) {
                in3 += 1;
            }
            if let Some(r) = rank {
                in5 += 1;
                rr += 1.0 / (r as f64 + 1.0);
            } else if show {
                println!(
                    "  miss ({label}): {:?} → {:?}, expected {}",
                    c.query, got, c.expect
                );
            }
        }
        let n = cases.len();
        // The judge's reads of this arm (the activity is newest first).
        let activity = app.judge_activity(1000);
        let reads = &activity[..activity.len() - before];
        let tokens: u64 = reads
            .iter()
            .filter_map(|a| a.usage.as_ref()?["input_tokens"].as_u64())
            .sum();
        println!(
            "| {label} | {first}/{n} | {in3}/{n} | {in5}/{n} | {:.2} | {} | {} | {} |",
            rr / n as f64,
            ms / n as u128,
            reads.len(),
            tokens / n as u64
        );
    }
    Ok(())
}
