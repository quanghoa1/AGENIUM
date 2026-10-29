//! `tool_search` with a model over an API (ADR-139): DeepSeek (OpenCode Go) given each of the 58 requests of
//! `agent/mcp/data/many-tools-queries.json` with the 123 tools of `many-tools.json`, the way `tool_search_vs_cli.py`
//! gives them to Claude Code. A request is right when the tool the model calls is the one that does it. Arms:
//!
//!   upfront  every tool listed
//!   bm25     tool_search + call_tool without the embedding model installed (words; the hashing stand-in)
//!   bge      with the embedding model (words + meaning)
//!   jev      with the embedding model and the judge (JEV_KEY)
//!
//! `--opaque`: the tools named tool_000… (descriptions kept), so names say nothing and only the search finds them.
//! Usage (build the server first: cargo build -p agenium-mcp --bin agenium-mcp-many-tools):
//!   OPENCODE_KEY=… [JEV_KEY=…] cargo run -p agenium-eval --example tool_search_api -- --models <root>/models
//!     [--arms upfront,bm25,bge,jev] [--opaque] [--model deepseek-v4.1-flash] [--parallel 6] [--out results.json]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agenium_core::Effort;
use agenium_llm::{
    ChatRequest, Message, Part, Provider, ProviderSpec, Role, SystemBlock, ToolSpec,
};
use agenium_tools::{AllowAll, Tool, ToolCtx};
use anyhow::{anyhow, Result};
use futures::stream::{self, StreamExt};
use serde_json::{json, Value};

const PROMPT: &str = "Task: {q}\n\nThis is a test environment: the team's MCP tools are simulated and nothing they do \
is real. Do the task with them: call the one tool that does it, once, with plausible arguments (invent any ids or values \
you need). Do not call any other tool after it and do not ask questions. Then reply DONE.";

#[derive(serde::Deserialize)]
struct Case {
    query: String,
    expect: String,
}

fn link_models(from: &Path, root: &Path) -> Result<()> {
    let to = root.join("models").join("bge-small-en-v1.5");
    std::fs::create_dir_all(&to)?;
    for e in std::fs::read_dir(from.join("bge-small-en-v1.5"))? {
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

/// One arm's tools: what the model gets, the capabilities line, and how to read the tool a call reached.
struct Arm {
    tools: Vec<Arc<dyn Tool>>,
    line: Option<String>,
}

async fn arm(name: &str, a: &Args, server: &Path, dir: &Path) -> Result<Arm> {
    let root = dir.join(name);
    let (app, _) = agenium_orchestrator::App::open(
        &root,
        agenium_orchestrator::AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
        },
    )?;
    if name == "bge" || name == "jev" {
        link_models(&a.models, &root)?;
    }
    if name == "jev" {
        let key = std::env::var("JEV_KEY").map_err(|_| anyhow!("the jev arm needs JEV_KEY"))?;
        app.set_judge(&agenium_orchestrator::judge::JudgeInput {
            base_url: "https://api.typesafe.ai".into(),
            model: "jev-1.14.0".into(),
            ..Default::default()
        })?;
        app.add_judge_key("eval", &key)?;
    }
    let app = Arc::new(app);
    let hub = agenium_mcp::McpHub::new(Arc::new(|_: &str| None), root.join("mcp"));
    let cfg = agenium_config::McpServerConfig {
        id: "many".into(),
        command: Some(server.display().to_string()),
        args: if a.opaque {
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
    let m = hub.tools_for(name, &[b], &[cfg]).await;
    if !m.problems.is_empty() {
        return Err(anyhow!("{:?}", m.problems));
    }
    let catalog = Arc::new(m.catalog(Arc::new(|_| true)));
    if name == "upfront" {
        let tools = catalog
            .listed()
            .await
            .into_iter()
            .map(|t| t as Arc<dyn Tool>)
            .collect();
        // Hub and app live as long as the process: leaked on purpose.
        std::mem::forget(hub);
        return Ok(Arm { tools, line: None });
    }
    let (tools, line) = app.mcp_search_offer(catalog.clone()).await;
    let _ = app.find_mcp_tools(catalog, "warm up", 1, true).await;
    std::mem::forget(hub);
    std::mem::forget(app);
    Ok(Arm {
        tools,
        line: Some(line),
    })
}

/// One request done: its index, the tool reached, tool calls, input and output tokens.
type Done = (usize, Option<String>, usize, u64, u64);

struct Args {
    models: PathBuf,
    arms: Vec<String>,
    opaque: bool,
    model: String,
    parallel: usize,
    out: Option<PathBuf>,
}

/// One request through one arm: the tool reached (the server's name), tool calls made, input and output tokens.
async fn run(
    p: &dyn Provider,
    arm: &Arm,
    model: &str,
    q: &str,
    ctx: &ToolCtx,
) -> (Option<String>, usize, u64, u64) {
    let mut system = vec![SystemBlock {
        text: "You are an agent. Use the tools you are given to do the user's task.".into(),
        cache: true,
    }];
    if let Some(l) = &arm.line {
        system.push(SystemBlock {
            text: l.clone(),
            cache: false,
        });
    }
    let mut specs: Vec<ToolSpec> = arm
        .tools
        .iter()
        .map(|t| ToolSpec {
            name: t.name().into(),
            description: t.description().into(),
            parameters: t.parameters(),
        })
        .collect();
    specs.sort_by(|a, b| a.name.cmp(&b.name));
    let mut messages = vec![Message::user_text(PROMPT.replace("{q}", q))];
    let (mut calls, mut input, mut output) = (0usize, 0u64, 0u64);
    for _ in 0..6 {
        let req = ChatRequest {
            model: model.into(),
            system: system.clone(),
            messages: messages.clone(),
            tools: specs.clone(),
            effort: Effort::LOW,
            max_tokens: 4000,
            cache_key: None,
            long_cache: false,
            tools_off: false,
        };
        let Ok(resp) = p.chat(&req, &|_| {}).await else {
            return (None, calls, input, output);
        };
        input += resp.usage.input + resp.usage.cache_read;
        output += resp.usage.output;
        let tc = resp.message.tool_calls();
        messages.push(resp.message.clone());
        if tc.is_empty() {
            return (None, calls, input, output);
        }
        let mut results = Vec::new();
        for c in tc {
            calls += 1;
            let reached = if c.name == "call_tool" {
                c.args["name"].as_str().map(str::to_owned)
            } else if c.name.starts_with("mcp__") {
                Some(c.name.clone())
            } else {
                None
            };
            if let Some(r) = reached {
                return (
                    Some(r.rsplit("__").next().unwrap_or(&r).to_owned()),
                    calls,
                    input,
                    output,
                );
            }
            let text = match arm.tools.iter().find(|t| t.name() == c.name) {
                Some(t) => match t.call(ctx, c.args.clone()).await {
                    Ok(o) => o.text,
                    Err(e) => format!("{}: {}", e.code, e.message),
                },
                None => format!("no tool {}", c.name),
            };
            results.push(Part::ToolResult {
                call_id: c.id,
                name: c.name,
                content: text,
                is_error: false,
            });
        }
        messages.push(Message {
            role: Role::User,
            parts: results,
        });
    }
    (None, calls, input, output)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() -> Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    let arg = |n: &str| {
        argv.iter()
            .position(|a| a == n)
            .and_then(|i| argv.get(i + 1).cloned())
    };
    let a = Args {
        models: PathBuf::from(arg("--models").ok_or_else(|| anyhow!("--models <dir>"))?),
        arms: arg("--arms")
            .unwrap_or_else(|| "upfront,bm25,bge,jev".into())
            .split(',')
            .map(str::to_owned)
            .collect(),
        opaque: argv.iter().any(|x| x == "--opaque"),
        model: arg("--model").unwrap_or_else(|| "deepseek-v4.1-flash".into()),
        parallel: arg("--parallel").and_then(|p| p.parse().ok()).unwrap_or(6),
        out: arg("--out").map(PathBuf::from),
    };
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let server = repo.join("target/debug").join(format!(
        "agenium-mcp-many-tools{}",
        std::env::consts::EXE_SUFFIX
    ));
    let key = std::env::var("OPENCODE_KEY").map_err(|_| anyhow!("set OPENCODE_KEY"))?;
    let provider = agenium_llm::build_provider(&ProviderSpec {
        id: "opencode-go".into(),
        kind: "openai_compat".into(),
        base_url: Some("https://opencode.ai/zen/go/v1".into()),
        api_key: Some(key),
        max_concurrency: 8,
        custom: Default::default(),
    })?;
    let cases: Vec<Case> = serde_json::from_str(&std::fs::read_to_string(
        repo.join("agent/mcp/data/many-tools-queries.json"),
    )?)?;
    // With opaque names, the tool that does a request is known by its place in the catalogue.
    let catalogue: Value = serde_json::from_str(&std::fs::read_to_string(
        repo.join("agent/mcp/data/many-tools.json"),
    )?)?;
    let opaque_name = |n: &str| -> String {
        let i = catalogue
            .as_array()
            .unwrap()
            .iter()
            .position(|t| t["name"] == n)
            .unwrap();
        format!("tool_{i:03}")
    };
    let tmp = tempfile::tempdir()?;
    let ctx = ToolCtx::new(tmp.path(), "eval", Arc::new(AllowAll));
    println!(
        "model {}, {} names",
        a.model,
        if a.opaque { "opaque" } else { "plain" }
    );
    println!(
        "\n| Arm | right tool | tool calls / run | input tokens / run | output tokens / run |"
    );
    println!("|---|---|---|---|---|");
    let mut all = Vec::new();
    for name in a.arms.clone() {
        let arm = arm(&name, &a, &server, tmp.path()).await?;
        let results: Vec<Done> = stream::iter(cases.iter().enumerate())
            .map(|(i, c)| {
                let (p, arm, ctx, model) = (provider.clone(), &arm, &ctx, a.model.clone());
                async move {
                    let (got, n, i_t, o_t) = run(p.as_ref(), arm, &model, &c.query, ctx).await;
                    (i, got, n, i_t, o_t)
                }
            })
            .buffer_unordered(a.parallel)
            .collect()
            .await;
        let n = cases.len();
        let mut right = 0;
        for (i, got, calls, it, ot) in &results {
            let want = if a.opaque {
                opaque_name(&cases[*i].expect)
            } else {
                cases[*i].expect.clone()
            };
            let ok = got.as_deref() == Some(want.as_str());
            right += usize::from(ok);
            all.push(
                json!({"arm": name, "query": cases[*i].query, "expect": want, "got": got, "ok": ok,
                "calls": calls, "input": it, "output": ot}),
            );
        }
        let sum = |f: fn(&Done) -> u64| results.iter().map(f).sum::<u64>() as f64 / n as f64;
        println!(
            "| {name} | {right}/{n} | {:.1} | {:.0} | {:.0} |",
            sum(|r| r.2 as u64),
            sum(|r| r.3),
            sum(|r| r.4)
        );
    }
    if let Some(o) = &a.out {
        std::fs::write(o, serde_json::to_string_pretty(&all)?)?;
    }
    // The servers' readers (leaked with the hubs) would keep the runtime from shutting down.
    std::process::exit(0)
}
