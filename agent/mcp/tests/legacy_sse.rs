//! The legacy HTTP+SSE transport (revision 2024-11-05) against a minimal server of that revision: it refuses the
//! Streamable HTTP POST, its GET stream names the POST endpoint, and its answers come back on the stream.

use std::collections::BTreeMap;
use std::time::Duration;

use agenium_mcp::{Client, ConnectSpec};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

async fn read_request(s: &mut TcpStream) -> Option<(String, String, String)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = s.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut first = head.lines().next()?.split(' ');
    let (method, path) = (first.next()?.to_owned(), first.next()?.to_owned());
    let len: usize = head
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|v| v.trim().parse().unwrap_or(0))
        })
        .unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let n = s.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Some((method, path, String::from_utf8_lossy(&body).to_string()))
}

fn answer(msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let result = match msg["method"].as_str()? {
        "initialize" => json!({"protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
                               "serverInfo": {"name": "legacy", "version": "1"}}),
        "tools/list" => json!({"tools": [{"name": "echo", "description": "Echo.",
                               "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}}]}),
        "tools/call" => json!({"content": [{"type": "text",
                               "text": msg["params"]["arguments"]["text"].as_str().unwrap_or("")}]}),
        _ => {
            return Some(
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "no"}}),
            )
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// The server: `/sse` (GET stream; a POST there is refused) and `/messages` (POST, answered on the stream).
async fn legacy_server() -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (to_stream, from_posts) = mpsc::unbounded_channel::<String>();
    let mut stream_rx = Some(from_posts);
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let Some((method, path, body)) = read_request(&mut s).await else {
                continue;
            };
            match (method.as_str(), path.split('?').next().unwrap_or("")) {
                ("GET", "/sse") => {
                    let Some(mut rx) = stream_rx.take() else {
                        continue;
                    };
                    tokio::spawn(async move {
                        let _ = s
                            .write_all(
                                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\n\
                                  event: endpoint\r\ndata: /messages?sessionId=1\r\n\r\n",
                            )
                            .await;
                        while let Some(event) = rx.recv().await {
                            if s.write_all(event.as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    });
                }
                ("POST", "/messages") => {
                    let _ = s
                        .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await;
                    if let Ok(msg) = serde_json::from_str::<Value>(&body) {
                        if let Some(a) = answer(&msg) {
                            let _ = to_stream.send(format!("event: message\ndata: {a}\n\n"));
                        }
                    }
                }
                _ => {
                    let _ = s
                        .write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await;
                }
            }
        }
    });
    format!("{base}/sse")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_legacy_sse_server_is_reached_after_the_post_is_refused() {
    let url = legacy_server().await;
    let c = Client::connect(
        &ConnectSpec::Http {
            url,
            headers: BTreeMap::new(),
            bearer: None,
        },
        Duration::from_secs(10),
    )
    .await
    .unwrap();
    assert_eq!(c.protocol, "2024-11-05");
    assert_eq!(c.server_info["name"], "legacy");
    let tools = c.list_tools(Duration::from_secs(10)).await.unwrap();
    assert_eq!(tools[0].name, "echo");
    let r = c
        .call_tool(
            "echo",
            json!({"text": "over the stream"}),
            Duration::from_secs(10),
        )
        .await
        .unwrap();
    assert_eq!(r.text, "over the stream");
}
