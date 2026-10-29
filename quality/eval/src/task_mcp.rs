//! The MCP server a task's agent is bound to (`mcp-search`, `mcp-fallback`; ADR-139): 123 tools shaped like real
//! servers' (`agent/mcp/data/many-tools.json`), served over Streamable HTTP from this process. Only
//! `releases_codename` knows the codename of a version, so the task is done only by finding that tool among the others.
//! For `mcp-fallback` the server starts with its own search (`find_tools`, `run_tool`), out of quota at the first use:
//! the call fails and the server lists every tool from then on, saying so (`notifications/tools/list_changed`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use agenium_core::AgentId;
use agenium_orchestrator::app::App;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TOOLS: &str = include_str!("../../../agent/mcp/data/many-tools.json");

/// The codename `releases_codename` answers; nothing else holds it.
pub const CODENAME: &str = "BLUE-HERON";

fn answer(name: &str, args: &Value) -> String {
    if name == "releases_codename" {
        format!(
            "The codename of version {} is {CODENAME}.",
            args["version"].as_str().unwrap_or("?")
        )
    } else {
        format!("{name} done")
    }
}

/// What the server answers to one JSON-RPC message: (content type, body), `None` for a notification.
fn handle(msg: &Value, small: &AtomicBool, all: &Value) -> Option<(&'static str, String)> {
    msg.get("id")?;
    let own = json!([
        {"name": "find_tools", "description": "Find this server's tools for a task (semantic search).",
         "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]},
         "annotations": {"readOnlyHint": true}},
        {"name": "run_tool", "description": "Run one of this server's tools found with find_tools.",
         "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "arguments": {"type": "object"}}, "required": ["name"]}}
    ]);
    let name = msg["params"]["name"].as_str().unwrap_or("");
    let args = &msg["params"]["arguments"];
    let ok = |result: Value| {
        Some((
            "application/json",
            json!({"jsonrpc": "2.0", "id": msg["id"], "result": result}).to_string(),
        ))
    };
    let text = |t: String| json!({"content": [{"type": "text", "text": t}]});
    match msg["method"].as_str().unwrap_or("") {
        "initialize" => ok(json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {"listChanged": true}},
            "serverInfo": {"name": "agenium-many-tools", "version": "1"}
        })),
        "tools/list" => ok(json!({"tools": if small.load(Ordering::SeqCst) { own } else { all.clone() }})),
        "tools/call" if name == "find_tools" && small.swap(false, Ordering::SeqCst) => {
            let r = json!({"jsonrpc": "2.0", "id": msg["id"], "result": {"content": [{"type": "text", "text": "the tool search is out of quota: every tool is listed now"}], "isError": true}});
            let notice = json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"});
            Some(("text/event-stream", format!("data: {notice}\n\ndata: {r}\n\n")))
        }
        "tools/call" if name == "run_tool" && small.load(Ordering::SeqCst) => {
            ok(text(answer(args["name"].as_str().unwrap_or(""), &args["arguments"])))
        }
        "tools/call" if !small.load(Ordering::SeqCst) && all.as_array()?.iter().any(|t| t["name"] == name) => {
            ok(text(answer(name, args)))
        }
        "tools/call" => Some((
            "application/json",
            json!({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32602, "message": format!("unknown tool {name}")}}).to_string(),
        )),
        "ping" => ok(json!({})),
        m => Some((
            "application/json",
            json!({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32601, "message": format!("no method {m}")}}).to_string(),
        )),
    }
}

/// Serves the tools on a free local port; the URL of the endpoint.
async fn serve(start_small: bool) -> anyhow::Result<String> {
    let small = Arc::new(AtomicBool::new(start_small));
    let all: Arc<Value> = Arc::new(serde_json::from_str(TOOLS)?);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/mcp", l.local_addr()?);
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            let (small, all) = (small.clone(), all.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                let body = loop {
                    let n = s.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..i]).to_ascii_lowercase();
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:")?.trim().parse().ok())
                            .unwrap_or(0);
                        while buf.len() < i + 4 + len {
                            let n = s.read(&mut tmp).await.unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&tmp[..n]);
                        }
                        break buf[i + 4..(i + 4 + len).min(buf.len())].to_vec();
                    }
                };
                let msg: Value = serde_json::from_slice(&body).unwrap_or_default();
                let out = match handle(&msg, &small, &all) {
                    None => "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
                    Some((ctype, body)) => format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    ),
                };
                let _ = s.write_all(out.as_bytes()).await;
                let _ = s.shutdown().await;
            });
        }
    });
    Ok(url)
}

/// Starts the task's MCP server and binds `agent` to it; nothing for other tasks.
pub async fn install(app: &App, agent: AgentId, task: &str) -> anyhow::Result<()> {
    let start_small = match task {
        "mcp-search" => false,
        "mcp-fallback" => true,
        _ => return Ok(()),
    };
    let url = serve(start_small).await?;
    app.put_mcp_server(agenium_config::McpServerConfig {
        id: "tools".into(),
        command: None,
        args: vec![],
        env: Default::default(),
        url: Some(url),
        headers: Default::default(),
        timeout_secs: 30,
        oauth: None,
    })?;
    app.set_agent_mcp(
        agent,
        &[agenium_mcp::Binding {
            server: "tools".into(),
            credentials: Default::default(),
            keys: Default::default(),
            tools: None,
        }],
    )?;
    Ok(())
}
