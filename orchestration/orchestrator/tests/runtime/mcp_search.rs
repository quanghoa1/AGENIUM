//! ADR-139: MCP tools found by search instead of listed, and a server whose tools change during a run.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use super::*;

const TOOLS: &str = include_str!("../../../../agent/mcp/data/many-tools.json");

/// A Streamable HTTP MCP server with the 123 tools of `many-tools.json`. `start_small`: it lists only its own search
/// (`find_tools`, `run_tool`) until the search is used, which fails (out of quota); from then on it lists every tool,
/// the failed call's event stream carrying `notifications/tools/list_changed` before the answer.
async fn many_tools(start_small: bool) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let small = Arc::new(AtomicBool::new(start_small));
    let all: serde_json::Value = serde_json::from_str(TOOLS).unwrap();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", l.local_addr().unwrap());
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else {
                return;
            };
            let (small, all) = (small.clone(), all.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
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
                            .find_map(|l| {
                                l.strip_prefix("content-length:")
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
                        break buf[i + 4..i + 4 + len].to_vec();
                    }
                };
                let msg: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                let reply = |ctype: &str, body: String| {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                let out = if msg.get("id").is_none() {
                    "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_owned()
                } else {
                    let own = json!([
                        {"name": "find_tools", "description": "Find this server's tools (semantic search).", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}}, "annotations": {"readOnlyHint": true}},
                        {"name": "run_tool", "description": "Run a tool found with find_tools.", "inputSchema": {"type": "object"}}
                    ]);
                    let name = msg["params"]["name"].as_str().unwrap_or("").to_owned();
                    let result = match msg["method"].as_str().unwrap_or("") {
                        "initialize" => {
                            json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {"listChanged": true}}, "serverInfo": {"name": "many"}})
                        }
                        "tools/list" => {
                            json!({"tools": if small.load(Ordering::SeqCst) { own } else { all.clone() }})
                        }
                        "tools/call"
                            if name == "find_tools" && small.swap(false, Ordering::SeqCst) =>
                        {
                            let r = json!({"jsonrpc": "2.0", "id": msg["id"], "result": {"content": [{"type": "text", "text": "the tool search is out of quota: every tool is listed now"}], "isError": true}});
                            let notice = json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"});
                            let stream = format!("data: {notice}\n\ndata: {r}\n\n");
                            let _ = s
                                .write_all(reply("text/event-stream", stream).as_bytes())
                                .await;
                            let _ = s.shutdown().await;
                            return;
                        }
                        "tools/call" if name == "releases_codename" => {
                            json!({"content": [{"type": "text", "text": format!("The codename of version {} is BLUE-HERON.", msg["params"]["arguments"]["version"].as_str().unwrap_or("?"))}]})
                        }
                        "tools/call" => {
                            json!({"content": [{"type": "text", "text": format!("{name} done")}]})
                        }
                        _ => json!({}),
                    };
                    reply(
                        "application/json",
                        json!({"jsonrpc": "2.0", "id": msg["id"], "result": result}).to_string(),
                    )
                };
                let _ = s.write_all(out.as_bytes()).await;
                let _ = s.shutdown().await;
            });
        }
    });
    url
}

fn bind(app: &agenium_orchestrator::App, agent: AgentId, url: String) {
    app.put_mcp_server(agenium_config::McpServerConfig {
        id: "many".into(),
        command: None,
        args: vec![],
        env: Default::default(),
        url: Some(url),
        headers: Default::default(),
        timeout_secs: 10,
        oauth: None,
    })
    .unwrap();
    app.set_agent_mcp(
        agent,
        &[agenium_mcp::Binding {
            server: "many".into(),
            credentials: Default::default(),
            keys: Default::default(),
            tools: None,
        }],
    )
    .unwrap();
}

fn names(req: &agenium_llm::ChatRequest) -> Vec<String> {
    req.tools.iter().map(|t| t.name.clone()).collect()
}

fn last_result(req: &agenium_llm::ChatRequest) -> (String, bool) {
    match &req.messages.last().unwrap().parts[0] {
        agenium_llm::Part::ToolResult {
            content, is_error, ..
        } => (content.clone(), *is_error),
        p => panic!("{p:?}"),
    }
}

/// 123 MCP tools are not listed: the agent finds the one it needs with `tool_search` and calls it with `call_tool`;
/// the request's tool list is the same in every step (the prompt-cache prefix holds).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_mcp_tools_are_found_by_search_and_called() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo_with_tickets(&app, &["PI-1"]);
    bind(&app, agent, many_tools(false).await);
    let first = Arc::new(Mutex::new(Vec::new()));
    let f1 = first.clone();
    p.push(Box::new(move |req| {
        let n = names(req);
        assert!(
            n.contains(&"tool_search".into()) && n.contains(&"call_tool".into()),
            "{n:?}"
        );
        assert!(
            !n.iter().any(|x| x.starts_with("mcp__")),
            "no MCP tool is listed: {n:?}"
        );
        let sys = &req.system[2].text;
        assert!(
            // Past 30 tools, a server's tools summed up by their names' first word, not named one by one.
            sys.contains("123 more MCP tools are not listed")
                && sys.contains("many: 123 tools (")
                && sys.contains("github 20")
                && !sys.contains("releases_codename"),
            "{sys}"
        );
        *f1.lock().unwrap() = n;
        Ok(calls_response(
            "",
            vec![(
                "tool_search",
                json!({"query": "what is the codename of version 4.2"}),
            )],
        ))
    }));
    let f2 = first.clone();
    p.push(Box::new(move |req| {
        assert_eq!(names(req), *f2.lock().unwrap());
        let (text, err) = last_result(req);
        assert!(
            !err && text.starts_with("## mcp__many__releases_codename"),
            "{text}"
        );
        Ok(calls_response(
            "",
            vec![(
                "call_tool",
                json!({"name": "mcp__many__releases_codename", "arguments": {"version": "4.2"}}),
            )],
        ))
    }));
    let f3 = first.clone();
    p.push(Box::new(move |req| {
        assert_eq!(names(req), *f3.lock().unwrap());
        let (text, err) = last_result(req);
        assert!(!err && text.contains("BLUE-HERON") && text.contains("Untrusted content"), "{text}");
        // The server's own name of a tool, with arguments as a JSON string, reaches it too.
        Ok(calls_response("", vec![("call_tool", json!({"name": "slack_post_message", "arguments": "{\"channel\": \"#x\", \"text\": \"hi\"}"}))]))
    }));
    p.push(Box::new(move |req| {
        let (text, err) = last_result(req);
        assert!(!err && text.contains("slack_post_message done"), "{text}");
        Ok(calls_response(
            "",
            vec![("call_tool", json!({"name": "mcp__many__no_such_tool"}))],
        ))
    }));
    p.push(Box::new(move |req| {
        let (text, err) = last_result(req);
        assert!(err && text.contains("find it with tool_search"), "{text}");
        Ok(calls_response(
            "",
            vec![(
                "submit_report",
                serde_json::to_value(report("AGN-1")).unwrap(),
            )],
        ))
    }));
    let s = rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    assert_eq!(s.outcome, "Completed");
    assert_eq!(p.remaining(), 0);

    // The agent's next run lists the MCP tool it used most; the rest stays behind the search.
    p.push(Box::new(|req| {
        let n = names(req);
        assert!(n.contains(&"mcp__many__releases_codename".into()), "{n:?}");
        assert!(n.contains(&"tool_search".into()));
        // A review keeps the 71 read-only tools: one listed, 70 found by search.
        assert!(
            req.system[2]
                .text
                .contains("70 more MCP tools are not listed"),
            "{}",
            req.system[2].text
        );
        Err(agenium_llm::LlmError::Unsupported("stop here".into()))
    }));
    let _ = rt.review(project, &key("AGN-1")).await;
    assert_eq!(p.remaining(), 0);
}

/// The user's case: a server with its own tool search lists two tools, so they are listed — with `tool_search` and
/// `call_tool`, since the server says its list may change. Its search out of quota, it lists all 123 tools: the run's
/// tool list stays as it was and the agent finds the tool it needs among the new ones.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_server_that_falls_back_to_all_its_tools_mid_run_is_followed() {
    let f = fixture();
    let p = Arc::new(ScriptedProvider::new("mock"));
    let (app, rt) = runtime(&f, p.clone(), Arc::new(AutoApprove));
    let (project, agent) = solo_with_tickets(&app, &["PI-1"]);
    bind(&app, agent, many_tools(true).await);
    let first = Arc::new(Mutex::new(Vec::new()));
    let f1 = first.clone();
    p.push(Box::new(move |req| {
        let n = names(req);
        for t in [
            "mcp__many__find_tools",
            "mcp__many__run_tool",
            "tool_search",
            "call_tool",
        ] {
            assert!(n.contains(&t.to_owned()), "{t} in {n:?}");
        }
        assert!(
            req.system[2]
                .text
                .contains("may change its tools during the run"),
            "{}",
            req.system[2].text
        );
        *f1.lock().unwrap() = n;
        Ok(calls_response(
            "",
            vec![(
                "mcp__many__find_tools",
                json!({"query": "release codename"}),
            )],
        ))
    }));
    let f2 = first.clone();
    p.push(Box::new(move |req| {
        let (text, err) = last_result(req);
        assert!(err && text.contains("out of quota"), "{text}");
        assert_eq!(
            names(req),
            *f2.lock().unwrap(),
            "the run's tool list does not change"
        );
        Ok(calls_response(
            "",
            vec![("mcp__many__run_tool", json!({"name": "releases_codename"}))],
        ))
    }));
    p.push(Box::new(move |req| {
        let (text, err) = last_result(req);
        assert!(
            err && text.contains("no longer offers run_tool") && text.contains("tool_search"),
            "{text}"
        );
        Ok(calls_response(
            "",
            vec![(
                "tool_search",
                json!({"query": "codename of a release version"}),
            )],
        ))
    }));
    p.push(Box::new(move |req| {
        let (text, err) = last_result(req);
        assert!(
            !err && text.starts_with("## mcp__many__releases_codename"),
            "{text}"
        );
        Ok(calls_response(
            "",
            vec![(
                "call_tool",
                json!({"name": "mcp__many__releases_codename", "arguments": {"version": "4.2"}}),
            )],
        ))
    }));
    p.push(Box::new(move |req| {
        let (text, err) = last_result(req);
        assert!(!err && text.contains("BLUE-HERON"), "{text}");
        Ok(calls_response(
            "",
            vec![(
                "submit_report",
                serde_json::to_value(report("AGN-1")).unwrap(),
            )],
        ))
    }));
    let s = rt.run_ticket(project, &key("AGN-1")).await.unwrap();
    assert_eq!(s.outcome, "Completed");
    assert_eq!(p.remaining(), 0);
    // The next run starts with the server's whole list: past the threshold, so found by search.
    p.push(Box::new(|req| {
        let n = names(req);
        assert!(
            !n.contains(&"mcp__many__slack_post_message".into()),
            "{n:?}"
        );
        assert!(n.contains(&"tool_search".into()) && n.contains(&"call_tool".into()));
        Err(agenium_llm::LlmError::Unsupported("stop here".into()))
    }));
    let _ = rt.review(project, &key("AGN-1")).await;
    assert_eq!(p.remaining(), 0);
}
