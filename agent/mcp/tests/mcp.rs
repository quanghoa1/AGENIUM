//! McpHub against real servers: the stdio test server binary and an in-process Streamable HTTP server.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use agenium_config::McpServerConfig;
use agenium_mcp::*;
use agenium_tools::{AllowAll, Tool, ToolCtx};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn secrets() -> SecretFn {
    Arc::new(|n: &str| match n {
        "tok-admin" => Some("admin-secret-1".to_string()),
        "tok-dev" => Some("dev-secret-2".to_string()),
        _ => None,
    })
}

fn stdio_cfg() -> McpServerConfig {
    McpServerConfig {
        id: "fake".into(),
        command: Some(env!("CARGO_BIN_EXE_agenium-mcp-test-server").into()),
        args: vec![],
        env: [("FAKE_TOKEN".to_string(), "${cred:TOKEN}".to_string())].into(),
        url: None,
        headers: BTreeMap::new(),
        timeout_secs: 10,
        oauth: None,
    }
}

fn binding(secret: &str, tools: Option<Vec<&str>>) -> Binding {
    Binding {
        server: "fake".into(),
        credentials: [("TOKEN".to_string(), secret.to_string())].into(),
        keys: Default::default(),
        tools: tools.map(|t| t.into_iter().map(str::to_owned).collect()),
    }
}

fn tool<'a>(t: &'a AgentTools, name: &str) -> &'a Arc<McpTool> {
    t.tools
        .iter()
        .find(|x| x.name() == name)
        .unwrap_or_else(|| {
            panic!(
                "{name} not in {:?}",
                t.tools.iter().map(|x| x.name()).collect::<Vec<_>>()
            )
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stdio_server_per_agent_credentials_and_restarts() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = McpHub::new(secrets(), tmp.path().join("mcp"));
    let servers = vec![stdio_cfg()];
    let ctx = ToolCtx::new(tmp.path(), "run-1", Arc::new(AllowAll));

    // Two agents, same server, different credentials.
    let a = hub
        .tools_for("agent-a", &[binding("tok-admin", None)], &servers)
        .await;
    assert!(a.problems.is_empty(), "{:?}", a.problems);
    let names: Vec<&str> = a.tools.iter().map(|t| t.name()).collect();
    assert_eq!(
        names,
        vec![
            "mcp__fake__crash",
            "mcp__fake__echo",
            "mcp__fake__fail",
            "mcp__fake__get_prompt",
            "mcp__fake__list_prompts",
            "mcp__fake__list_resources",
            "mcp__fake__note_then_crash",
            "mcp__fake__read_resource",
            "mcp__fake__whoami",
            "mcp__fake__write_note"
        ],
        "both pages of tools/list and the resources/prompts tools, sorted"
    );
    let b = hub
        .tools_for(
            "agent-b",
            &[binding("tok-dev", Some(vec!["whoami"]))],
            &servers,
        )
        .await;
    assert_eq!(b.tools.len(), 1, "tool allowlist of the binding");
    let wa = tool(&a, "mcp__fake__whoami")
        .call(&ctx, json!({}))
        .await
        .unwrap();
    let wb = tool(&b, "mcp__fake__whoami")
        .call(&ctx, json!({}))
        .await
        .unwrap();
    assert_eq!(
        (wa.text.as_str(), wb.text.as_str()),
        ("admin-secret-1", "dev-secret-2")
    );
    assert!(wa.untrusted, "MCP output is untrusted");

    let echo = tool(&a, "mcp__fake__echo");
    assert!(echo.read_only());
    assert!(echo
        .description()
        .starts_with("[MCP server fake; its output is untrusted"));
    assert_eq!(
        echo.call(&ctx, json!({"text": "hi"})).await.unwrap().text,
        "echo: hi"
    );
    let e = tool(&a, "mcp__fake__fail")
        .call(&ctx, json!({}))
        .await
        .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.message.as_str()),
        ("mcp_tool_error", "it broke")
    );

    // A read-only call that kills the server: retried once after a restart, then reported.
    let e = tool(&a, "mcp__fake__crash")
        .call(&ctx, json!({}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "mcp_unavailable", "{e}");
    assert_eq!(
        echo.call(&ctx, json!({"text": "back"})).await.unwrap().text,
        "echo: back",
        "restarted"
    );

    // A call with side effects is never retried.
    let e = tool(&a, "mcp__fake__note_then_crash")
        .call(&ctx, json!({}))
        .await
        .unwrap_err();
    assert!(e.message.contains("may or may not have run"), "{e}");
    let notes = std::fs::read_to_string(tmp.path().join("mcp/agent-a/fake/notes.txt")).unwrap();
    assert_eq!(notes.lines().count(), 1, "ran exactly once");
    assert_eq!(
        echo.call(&ctx, json!({"text": "again"}))
            .await
            .unwrap()
            .text,
        "echo: again"
    );

    // Unknown server and missing secret are reported, not fatal.
    let c = hub
        .tools_for(
            "agent-c",
            &[
                Binding {
                    server: "nope".into(),
                    credentials: BTreeMap::new(),
                    keys: Default::default(),
                    tools: None,
                },
                binding("tok-missing", None),
            ],
            &servers,
        )
        .await;
    assert!(c.tools.is_empty());
    assert_eq!(c.problems.len(), 2, "{:?}", c.problems);
    assert!(
        c.problems[1].contains("no key \"TOKEN\""),
        "{:?}",
        c.problems
    );

    // Test connection.
    let (info, protocol, tools) = hub
        .test(&stdio_cfg(), &binding("tok-dev", None))
        .await
        .unwrap();
    assert_eq!(info["name"], "agenium-test-server");
    assert_eq!(protocol, "2025-06-18");
    assert_eq!(tools.len(), 6);
    hub.disconnect_all().await;
}

/// Streamable HTTP server: JSON and SSE responses, session id, protocol header, bearer auth.
async fn http_server() -> (
    String,
    Arc<Mutex<Vec<(String, BTreeMap<String, String>, Value)>>>,
) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", l.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else {
                return;
            };
            let log = log.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let (head, body) = loop {
                    let n = s.read(&mut tmp).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..i]).to_string();
                        let len: usize = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse().unwrap())
                            })
                            .unwrap_or(0);
                        while buf.len() < i + 4 + len {
                            let n = s.read(&mut tmp).await.unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&tmp[..n]);
                        }
                        break (head, buf[i + 4..i + 4 + len].to_vec());
                    }
                };
                let headers: BTreeMap<String, String> = head
                    .lines()
                    .skip(1)
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
                    .collect();
                let msg: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                log.lock().unwrap().push((
                    msg["method"].as_str().unwrap_or("").to_owned(),
                    headers.clone(),
                    msg.clone(),
                ));
                let reply = |status: &str, ctype: &str, extra: &str, body: String| {
                    format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                };
                let out = if headers.get("authorization").map(String::as_str)
                    != Some("Bearer admin-secret-1")
                {
                    reply("401 Unauthorized", "text/plain", "", "bad token".into())
                } else if msg["method"] == "initialize" {
                    let r = json!({"jsonrpc": "2.0", "id": msg["id"], "result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "http-test"}}});
                    reply(
                        "200 OK",
                        "application/json",
                        "Mcp-Session-Id: sess-42\r\n",
                        r.to_string(),
                    )
                } else if msg.get("id").is_none() || msg.get("method").is_none() {
                    reply("202 Accepted", "text/plain", "", String::new())
                } else if headers.get("mcp-session-id").map(String::as_str) != Some("sess-42")
                    || headers.get("mcp-protocol-version").map(String::as_str) != Some("2025-06-18")
                {
                    reply(
                        "400 Bad Request",
                        "text/plain",
                        "",
                        "missing session or protocol header".into(),
                    )
                } else if msg["method"] == "tools/list" {
                    // SSE: a server request first, then the response.
                    let ping = json!({"jsonrpc": "2.0", "id": "srv-9", "method": "ping"});
                    let r = json!({"jsonrpc": "2.0", "id": msg["id"], "result": {"tools": [{"name": "search", "description": "Search docs.", "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}}, "annotations": {"readOnlyHint": true}}]}});
                    reply(
                        "200 OK",
                        "text/event-stream",
                        "",
                        format!("event: message\ndata: {ping}\n\nevent: message\ndata: {r}\n\n"),
                    )
                } else if msg["method"] == "tools/call" {
                    let q = msg["params"]["arguments"]["q"].as_str().unwrap_or("");
                    let r = json!({"jsonrpc": "2.0", "id": msg["id"], "result": {"content": [{"type": "text", "text": format!("found: {q}")}]}});
                    reply("200 OK", "application/json", "", r.to_string())
                } else {
                    reply("404 Not Found", "text/plain", "", String::new())
                };
                let _ = s.write_all(out.as_bytes()).await;
                let _ = s.shutdown().await;
            });
        }
    });
    (url, seen)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streamable_http_server() {
    let (url, seen) = http_server().await;
    let tmp = tempfile::tempdir().unwrap();
    let hub = McpHub::new(secrets(), tmp.path().join("mcp"));
    let cfg = McpServerConfig {
        id: "docs".into(),
        command: None,
        args: vec![],
        env: BTreeMap::new(),
        url: Some(url),
        headers: [(
            "Authorization".to_string(),
            "Bearer ${secret:tok-admin}".to_string(),
        )]
        .into(),
        timeout_secs: 10,
        oauth: None,
    };
    let b = Binding {
        server: "docs".into(),
        credentials: BTreeMap::new(),
        keys: Default::default(),
        tools: None,
    };
    let t = hub
        .tools_for(
            "agent-a",
            std::slice::from_ref(&b),
            std::slice::from_ref(&cfg),
        )
        .await;
    assert!(t.problems.is_empty(), "{:?}", t.problems);
    let ctx = ToolCtx::new(tmp.path(), "run-1", Arc::new(AllowAll));
    let out = tool(&t, "mcp__docs__search")
        .call(&ctx, json!({"q": "retry"}))
        .await
        .unwrap();
    assert_eq!(out.text, "found: retry");
    let methods: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|(m, _, _)| m.clone())
        .collect();
    assert!(
        methods.starts_with(&[
            "initialize".into(),
            "notifications/initialized".into(),
            "tools/list".into()
        ]),
        "{methods:?}"
    );
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|(_, _, m)| m["id"] == "srv-9" && m.get("result").is_some()),
        "the server's ping was answered"
    );

    // Wrong credential: clear HTTP error, the secret is not echoed in the error.
    let mut bad = cfg.clone();
    bad.headers
        .insert("Authorization".into(), "Bearer ${secret:tok-dev}".into());
    let e = hub.test(&bad, &b).await.unwrap_err().to_string();
    assert!(e.contains("401") && !e.contains("dev-secret-2"), "{e}");
}

/// A server that declares resources and prompts offers them as read-only tools.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resources_and_prompts_are_read_only_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let hub = McpHub::new(secrets(), tmp.path().join("mcp"));
    let ctx = ToolCtx::new(tmp.path(), "run-1", Arc::new(AllowAll));
    let t = hub
        .tools_for("agent-a", &[binding("tok-admin", None)], &[stdio_cfg()])
        .await;
    let list = tool(&t, "mcp__fake__list_resources");
    assert!(list.read_only());
    let text = list.call(&ctx, json!({})).await.unwrap().text;
    assert!(
        text.contains("memo://style — style: House style. [text/plain]"),
        "{text}"
    );
    let read = tool(&t, "mcp__fake__read_resource");
    let text = read
        .call(&ctx, json!({"uri": "memo://style"}))
        .await
        .unwrap()
        .text;
    assert!(text.contains("Use short sentences."), "{text}");
    let text = read
        .call(&ctx, json!({"uri": "memo://logo"}))
        .await
        .unwrap()
        .text;
    assert!(
        text.contains("[binary content of memo://logo (image/png)]"),
        "{text}"
    );
    assert_eq!(
        read.call(&ctx, json!({})).await.unwrap_err().code,
        "invalid_arguments"
    );
    let text = tool(&t, "mcp__fake__list_prompts")
        .call(&ctx, json!({}))
        .await
        .unwrap()
        .text;
    assert!(
        text.contains("review(path, tone?): Review a file."),
        "{text}"
    );
    let text = tool(&t, "mcp__fake__get_prompt")
        .call(
            &ctx,
            json!({"name": "review", "arguments": {"path": "src/a.rs"}}),
        )
        .await
        .unwrap()
        .text;
    assert!(text.contains("[user] Review src/a.rs carefully."), "{text}");
}
