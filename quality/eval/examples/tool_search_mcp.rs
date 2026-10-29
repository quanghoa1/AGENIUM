//! Agenium's `tool_search` and `call_tool` over the 123 tools of `agent/mcp/data/many-tools.json`, served as an MCP
//! stdio server, so a vendor CLI can be measured with Agenium's search against its own (`tool_search_vs_cli.py`;
//! ADR-139). The tools are the ones a run gets (words, meaning and — with `JEV_KEY` — the judge); every call is
//! appended to `TS_LOG` as a JSON line. `--line` prints the capabilities line a run gets (the tools' names) and exits.
//!
//! Environment: `MANY_TOOLS` (the `agenium-mcp-many-tools` binary; default next to this one's target folder),
//! `MODELS` (a folder holding `bge-small-en-v1.5`), `JEV_KEY` (optional), `TS_LOG` (optional).

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agenium_tools::{AllowAll, ToolCtx};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};

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

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let line_only = std::env::args().any(|a| a == "--line");
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let server = std::env::var("MANY_TOOLS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            repo.join("target/debug").join(format!(
                "agenium-mcp-many-tools{}",
                std::env::consts::EXE_SUFFIX
            ))
        });
    let tmp = tempfile::tempdir()?;
    let root = tmp.path().join("root");
    let (app, _) = agenium_orchestrator::App::open(
        &root,
        agenium_orchestrator::AppOptions {
            keyring: Arc::new(agenium_secrets::MemoryKeyring::default()),
            checks: Arc::new(agenium_orchestrator::NoCheckFactory),
        },
    )?;
    if let Ok(m) = std::env::var("MODELS") {
        link_models(Path::new(&m), &root)?;
    }
    if let Ok(key) = std::env::var("JEV_KEY") {
        app.set_judge(&agenium_orchestrator::judge::JudgeInput {
            base_url: "https://api.typesafe.ai".into(),
            model: "jev-1.14.0".into(),
            ..Default::default()
        })?;
        app.add_judge_key("eval", &key)?;
    }
    let app = Arc::new(app);
    let hub = agenium_mcp::McpHub::new(Arc::new(|_: &str| None), tmp.path().join("mcp"));
    let cfg = agenium_config::McpServerConfig {
        id: "many".into(),
        command: Some(server.display().to_string()),
        args: vec![],
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
    let (tools, line) = app.mcp_search_offer(catalog.clone()).await;
    if line_only {
        println!("{line}");
        return Ok(());
    }
    // The catalogue embedded before the first search, as a long-running app has it.
    let _ = app.find_mcp_tools(catalog, "warm up", 1, true).await;
    let ctx = ToolCtx::new(tmp.path(), "eval", Arc::new(AllowAll));
    let log = std::env::var("TS_LOG").ok();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for l in stdin.lock().lines() {
        let Ok(l) = l else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&l) else {
            continue;
        };
        let id = msg["id"].clone();
        if id.is_null() {
            continue;
        }
        let result = match msg["method"].as_str().unwrap_or("") {
            "initialize" => json!({
                "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "agenium-tool-search", "version": "1"}
            }),
            "ping" => json!({}),
            "tools/list" => json!({"tools": tools.iter().map(|t| json!({
                "name": t.name(), "description": t.description(), "inputSchema": t.parameters()
            })).collect::<Vec<_>>()}),
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or("");
                let args = msg["params"]["arguments"].clone();
                if let Some(p) = &log {
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(p)
                    {
                        let _ = writeln!(f, "{}", json!({"tool": name, "args": args}));
                    }
                }
                match tools.iter().find(|t| t.name() == name) {
                    Some(t) => match t.call(&ctx, args).await {
                        Ok(o) => json!({"content": [{"type": "text", "text": o.text}]}),
                        Err(e) => {
                            json!({"content": [{"type": "text", "text": format!("{}: {}", e.code, e.message)}], "isError": true})
                        }
                    },
                    None => {
                        json!({"content": [{"type": "text", "text": format!("no tool {name}")}], "isError": true})
                    }
                }
            }
            m => {
                writeln!(
                    out,
                    "{}",
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("no method {m}")}})
                )?;
                out.flush()?;
                continue;
            }
        };
        writeln!(
            out,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "result": result})
        )?;
        out.flush()?;
    }
    Ok(())
}
