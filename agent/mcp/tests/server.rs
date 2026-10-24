//! The MCP server (ADR-56) seen through Agenium's own HTTP client, plus raw requests a client must not get through.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agenium_mcp::server::{Server, ToolHost};
use agenium_mcp::{CallResult, Client, ConnectSpec, McpError, ToolInfo};
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const T: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Host {
    calls: Mutex<Vec<(String, Value)>>,
}

#[async_trait]
impl ToolHost for Host {
    fn tools(&self) -> Vec<ToolInfo> {
        vec![
            ToolInfo {
                name: "submit_report".into(),
                description: "Report the result.".into(),
                input_schema: json!({"type": "object", "properties": {"summary": {"type": "string"}}}),
                read_only: false,
            },
            ToolInfo {
                name: "status".into(),
                description: "Ticket status.".into(),
                input_schema: json!({"type": "object"}),
                read_only: true,
            },
        ]
    }

    async fn call(&self, name: &str, args: Value) -> CallResult {
        self.calls.lock().unwrap().push((name.into(), args.clone()));
        match name {
            "status" => CallResult {
                text: "InProgress".into(),
                is_error: false,
            },
            _ if args["summary"].is_string() => CallResult {
                text: "report accepted".into(),
                is_error: false,
            },
            _ => CallResult {
                text: "summary is required".into(),
                is_error: true,
            },
        }
    }
}

fn spec(server: &Server, auth: Option<String>) -> ConnectSpec {
    let mut headers = BTreeMap::new();
    if let Some(a) = auth {
        headers.insert("Authorization".into(), a);
    }
    ConnectSpec::Http {
        url: server.url(),
        headers,
    }
}

#[tokio::test]
async fn client_lists_and_calls_tools() {
    let host = Arc::new(Host::default());
    let server = Server::start(host.clone(), "agenium").await.unwrap();
    let c = Client::connect(&spec(&server, Some(server.authorization())), T)
        .await
        .unwrap();
    let tools = c.list_tools(T).await.unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["submit_report", "status"]);
    assert!(tools[1].read_only && !tools[0].read_only);

    let ok = c
        .call_tool("submit_report", json!({"summary": "done"}), T)
        .await
        .unwrap();
    assert_eq!((ok.text.as_str(), ok.is_error), ("report accepted", false));
    let bad = c.call_tool("submit_report", json!({}), T).await.unwrap();
    assert!(bad.is_error);
    let st = c.call_tool("status", Value::Null, T).await.unwrap();
    assert_eq!(st.text, "InProgress");
    assert!(matches!(
        c.call_tool("rm_rf", json!({}), T).await,
        Err(McpError::Rpc { code: -32602, .. })
    ));
    c.ping(T).await.unwrap();
    let calls = host.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 3, "unknown tools never reach the host");
    assert_eq!(calls[2], ("status".into(), json!({})));
}

#[tokio::test]
async fn token_is_required() {
    let host = Arc::new(Host::default());
    let server = Server::start(host.clone(), "agenium").await.unwrap();
    for auth in [None, Some("Bearer wrong".to_owned())] {
        match Client::connect(&spec(&server, auth), T).await {
            Err(McpError::Http { status: 401, .. }) => {}
            other => panic!("expected 401, got {:?}", other.map(|_| ())),
        }
    }
    assert!(host.calls.lock().unwrap().is_empty());
}

/// Sends raw HTTP and returns the status line.
async fn raw(server: &Server, request: String) -> String {
    let addr = server
        .url()
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(T, s.read_to_end(&mut out)).await;
    String::from_utf8_lossy(&out)
        .lines()
        .next()
        .unwrap_or("")
        .to_owned()
}

#[tokio::test]
async fn rejects_foreign_origins_paths_and_methods() {
    let server = Server::start(Arc::new(Host::default()), "agenium")
        .await
        .unwrap();
    let auth = server.authorization();
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
    let post = |path: &str, extra: &str| {
        format!(
            "POST {path} HTTP/1.1\r\nHost: x\r\nAuthorization: {auth}\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    };
    assert_eq!(raw(&server, post("/mcp", "")).await, "HTTP/1.1 200 OK");
    assert_eq!(
        raw(&server, post("/mcp", "Origin: https://evil.example\r\n")).await,
        "HTTP/1.1 403 Forbidden"
    );
    assert_eq!(
        raw(&server, post("/other", "")).await,
        "HTTP/1.1 404 Not Found"
    );
    assert_eq!(
        raw(
            &server,
            format!("GET /mcp HTTP/1.1\r\nAuthorization: {auth}\r\nConnection: close\r\n\r\n")
        )
        .await,
        "HTTP/1.1 405 Method Not Allowed"
    );
    // A notification gets 202 and no body.
    let note = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    assert_eq!(
        raw(
            &server,
            format!(
                "POST /mcp HTTP/1.1\r\nAuthorization: {auth}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{note}",
                note.len()
            )
        )
        .await,
        "HTTP/1.1 202 Accepted"
    );
}
