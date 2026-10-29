//! The MCP server a task's agent is bound to (`mcp-search`, `mcp-fallback`, `mcp-fallback-late`; ADR-139): 123 tools
//! shaped like real servers' (`agent/mcp/data/many-tools.json`), served over Streamable HTTP from this process. Only
//! `releases_codename` knows the codename of a version, so the task is done only by finding that tool among the others.
//!
//! The fallbacks start as a server with its own tool search (`find_tools`, `run_tool`, the way a server searching its
//! tools with Jev offers them) whose search runs out of quota: the call fails and the server lists every tool from then
//! on. `mcp-fallback`: out of quota at the first use, and the server says so (`listChanged`, the notice).
//! `mcp-fallback-late`: its search and runs work twice first, and the server changes its tools without a word (no
//! `listChanged`, no notice).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use agenium_core::AgentId;
use agenium_orchestrator::app::App;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TOOLS: &str = include_str!("../../../agent/mcp/data/many-tools.json");

/// The codename of version 4.2 `releases_codename` answers; nothing else holds it.
pub const CODENAME: &str = "BLUE-HERON";
/// The codename of version 5.0 (`mcp-fallback-late` asks for both).
pub const CODENAME_5: &str = "RED-KITE";

fn answer(name: &str, args: &Value) -> String {
    if name != "releases_codename" {
        return format!("{name} done");
    }
    let v = args["version"].as_str().unwrap_or("?");
    match v.trim_start_matches('v') {
        "4.2" | "4.2.0" => format!("The codename of version {v} is {CODENAME}."),
        "5.0" | "5" | "5.0.0" => format!("The codename of version {v} is {CODENAME_5}."),
        _ => format!("Version {v} has no codename."),
    }
}

/// How the server behaves.
struct Server {
    all: Value,
    /// Only its own search is listed.
    small: AtomicBool,
    /// Calls of its own search (`find_tools`, `run_tool`) that work before its quota is out.
    quota: AtomicU32,
    /// It declares `listChanged` and sends the notice when its tools change.
    announce: bool,
}

impl Server {
    fn own() -> Value {
        json!([
            {"name": "find_tools", "description": "Find this server's tools for a task (semantic search).",
             "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]},
             "annotations": {"readOnlyHint": true}},
            {"name": "run_tool", "description": "Run one of this server's tools found with find_tools.",
             "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}, "arguments": {"type": "object"}}, "required": ["name"]}}
        ])
    }

    /// The search of a server that finds its tools by meaning: its best few by the words of the query.
    fn find(&self, query: &str) -> String {
        let q = query.to_lowercase();
        let mut hits: Vec<(usize, String)> = self
            .all
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                let text = format!("{} {}", t["name"], t["description"])
                    .to_lowercase()
                    .replace('_', " ");
                let score = q
                    .split_whitespace()
                    .filter(|w| w.len() > 2 && text.contains(*w))
                    .count();
                (
                    score,
                    format!(
                        "{}: {}",
                        t["name"].as_str().unwrap_or(""),
                        t["description"].as_str().unwrap_or("")
                    ),
                )
            })
            .filter(|(s, _)| *s > 0)
            .collect();
        hits.sort_by_key(|h| std::cmp::Reverse(h.0));
        let found: Vec<String> = hits.into_iter().take(5).map(|(_, l)| l).collect();
        format!("Run one with run_tool:\n{}", found.join("\n"))
    }

    /// What the server answers to one JSON-RPC message: (content type, body), `None` for a notification.
    fn handle(&self, msg: &Value) -> Option<(&'static str, String)> {
        msg.get("id")?;
        let name = msg["params"]["name"].as_str().unwrap_or("");
        let args = &msg["params"]["arguments"];
        let ok = |result: Value| {
            Some((
                "application/json",
                json!({"jsonrpc": "2.0", "id": msg["id"], "result": result}).to_string(),
            ))
        };
        let text = |t: String| json!({"content": [{"type": "text", "text": t}]});
        let small = self.small.load(Ordering::SeqCst);
        match msg["method"].as_str().unwrap_or("") {
            "initialize" => ok(json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {"listChanged": self.announce}},
                "serverInfo": {"name": "agenium-many-tools", "version": "1"}
            })),
            "tools/list" => ok(json!({"tools": if small { Self::own() } else { self.all.clone() }})),
            "tools/call" if small && (name == "find_tools" || name == "run_tool") => {
                let left = self.quota.load(Ordering::SeqCst);
                if left > 0 {
                    self.quota.store(left - 1, Ordering::SeqCst);
                    return ok(text(if name == "find_tools" {
                        self.find(args["query"].as_str().unwrap_or(""))
                    } else {
                        answer(args["name"].as_str().unwrap_or(""), &args["arguments"])
                    }));
                }
                self.small.store(false, Ordering::SeqCst);
                let r = json!({"jsonrpc": "2.0", "id": msg["id"], "result": {"content": [{"type": "text", "text": "the tool search is out of quota"}], "isError": true}});
                if self.announce {
                    let notice = json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"});
                    Some(("text/event-stream", format!("data: {notice}\n\ndata: {r}\n\n")))
                } else {
                    Some(("application/json", r.to_string()))
                }
            }
            "tools/call" if !small && self.all.as_array()?.iter().any(|t| t["name"] == name) => {
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
}

/// Serves the tools on a free local port; the URL of the endpoint.
async fn serve(server: Server) -> anyhow::Result<String> {
    let server = Arc::new(server);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/mcp", l.local_addr()?);
    tokio::spawn(async move {
        while let Ok((mut s, _)) = l.accept().await {
            let server = server.clone();
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
                let out = match server.handle(&msg) {
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
    let (small, quota, announce) = match task {
        "mcp-search" => (false, 0, true),
        "mcp-fallback" => (true, 0, true),
        "mcp-fallback-late" => (true, 2, false),
        _ => return Ok(()),
    };
    let url = serve(Server {
        all: serde_json::from_str(TOOLS)?,
        small: AtomicBool::new(small),
        quota: AtomicU32::new(quota),
        announce,
    })
    .await?;
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
