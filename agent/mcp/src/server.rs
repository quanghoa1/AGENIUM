//! MCP server side, the smallest Agenium needs: Agenium's own tools (the protocol tools of a run) offered to a vendor
//! CLI driven by the CliBackend (ADR-56). Streamable HTTP on the loopback interface, one random bearer token per
//! server, plain JSON responses (no SSE stream, no server-initiated requests), no session state.
//!
//! The HTTP/1.1 handling is deliberately minimal (Content-Length bodies, keep-alive, no chunked requests): the only
//! clients are the CLIs Agenium itself starts.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::client::{CallResult, ToolInfo, KNOWN_VERSIONS, PROTOCOL_VERSION};

/// Endpoint path of the server.
pub const PATH: &str = "/mcp";
/// Endpoint of permission checks from a CLI's pre-tool hook (same token).
pub const HOOK_PATH: &str = "/hook";
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// The tools a server offers and how a call is executed.
#[async_trait]
pub trait ToolHost: Send + Sync {
    fn tools(&self) -> Vec<ToolInfo>;
    async fn call(&self, name: &str, args: Value) -> CallResult;
    /// `POST /hook`: a vendor CLI's pre-tool hook asks whether a tool call may run (CliBackend). The default
    /// refuses everything.
    async fn hook(&self, _payload: Value) -> Value {
        json!({"decision": "deny", "reason": "no permission handler on this server"})
    }
}

/// A running server; dropping it stops accepting connections.
pub struct Server {
    addr: SocketAddr,
    token: String,
    task: JoinHandle<()>,
}

impl Server {
    /// Binds `127.0.0.1` on a free port. `name` is the `serverInfo.name` clients see.
    pub async fn start(host: Arc<dyn ToolHost>, name: &str) -> std::io::Result<Server> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let token: String = rand::random::<[u8; 32]>()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let shared = Arc::new(Shared {
            host,
            token: token.clone(),
            name: name.to_owned(),
        });
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let s = shared.clone();
                tokio::spawn(async move {
                    let _ = connection(stream, &s).await;
                });
            }
        });
        Ok(Server { addr, token, task })
    }

    pub fn url(&self) -> String {
        format!("http://{}{PATH}", self.addr)
    }

    /// URL of the permission-check endpoint.
    pub fn hook_url(&self) -> String {
        format!("http://{}{HOOK_PATH}", self.addr)
    }

    /// Value of the `Authorization` header every request must carry.
    pub fn authorization(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Shared {
    host: Arc<dyn ToolHost>,
    token: String,
    name: String,
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Reads one request; `Ok(None)` when the client closed the connection.
async fn read_request<R: AsyncBufReadExt + Unpin>(r: &mut R) -> std::io::Result<Option<Request>> {
    let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_owned());
    let mut line = String::new();
    if r.read_line(&mut line).await? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(path)) = (parts.next(), parts.next()) else {
        return Err(bad("malformed request line"));
    };
    let (method, path) = (method.to_owned(), path.to_owned());
    let mut headers = Vec::new();
    let mut size = line.len();
    loop {
        line.clear();
        if r.read_line(&mut line).await? == 0 {
            return Err(bad("connection closed in the headers"));
        }
        size += line.len();
        if size > MAX_HEADER_BYTES {
            return Err(bad("headers too large"));
        }
        let l = line.trim_end_matches(['\r', '\n']);
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
        }
    }
    let mut req = Request {
        method,
        path,
        headers,
        body: Vec::new(),
    };
    if req.header("transfer-encoding").is_some() {
        return Err(bad("chunked request bodies are not supported"));
    }
    let len: usize = req
        .header("content-length")
        .map(|v| v.parse().map_err(|_| bad("bad content-length")))
        .transpose()?
        .unwrap_or(0);
    if len > MAX_BODY_BYTES {
        return Err(bad("body too large"));
    }
    req.body = vec![0; len];
    r.read_exact(&mut req.body).await?;
    Ok(Some(req))
}

async fn respond<W: AsyncWriteExt + Unpin>(
    w: &mut W,
    status: &str,
    body: Option<&Value>,
) -> std::io::Result<()> {
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let ctype = if body.is_empty() {
        ""
    } else {
        "Content-Type: application/json\r\n"
    };
    let head = format!(
        "HTTP/1.1 {status}\r\n{ctype}Content-Length: {}\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    );
    w.write_all(head.as_bytes()).await?;
    w.write_all(body.as_bytes()).await?;
    w.flush().await
}

/// Constant-time comparison of the presented token.
fn token_ok(presented: Option<&str>, token: &str) -> bool {
    let Some(p) = presented.and_then(|v| v.strip_prefix("Bearer ")) else {
        return false;
    };
    let (a, b) = (p.trim().as_bytes(), token.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Browsers send `Origin`; only a loopback origin may reach the server (DNS-rebinding guard of the MCP spec).
fn origin_ok(origin: Option<&str>) -> bool {
    origin.is_none_or(|o| {
        let authority = o
            .split("://")
            .nth(1)
            .unwrap_or(o)
            .split('/')
            .next()
            .unwrap_or("");
        authority.starts_with("[::1]")
            || matches!(authority.split(':').next(), Some("127.0.0.1" | "localhost"))
    })
}

async fn connection(stream: TcpStream, s: &Shared) -> std::io::Result<()> {
    let (r, mut w) = stream.into_split();
    let mut r = BufReader::new(r);
    while let Some(req) = read_request(&mut r).await? {
        let close = req
            .header("connection")
            .is_some_and(|v| v.eq_ignore_ascii_case("close"));
        let path = req.path.split('?').next().unwrap_or("");
        if path != PATH && path != HOOK_PATH {
            respond(&mut w, "404 Not Found", None).await?;
        } else if !origin_ok(req.header("origin")) {
            respond(&mut w, "403 Forbidden", None).await?;
        } else if !token_ok(req.header("authorization"), &s.token) {
            respond(&mut w, "401 Unauthorized", None).await?;
        } else if path == HOOK_PATH {
            match (
                req.method.as_str(),
                serde_json::from_slice::<Value>(&req.body),
            ) {
                ("POST", Ok(payload)) => {
                    let answer = s.host.hook(payload).await;
                    respond(&mut w, "200 OK", Some(&answer)).await?
                }
                ("POST", Err(_)) => respond(&mut w, "400 Bad Request", None).await?,
                _ => respond(&mut w, "405 Method Not Allowed", None).await?,
            }
        } else {
            match req.method.as_str() {
                "POST" => match serde_json::from_slice::<Value>(&req.body) {
                    Ok(msg) => match handle(s, &msg).await {
                        Some(answer) => respond(&mut w, "200 OK", Some(&answer)).await?,
                        None => respond(&mut w, "202 Accepted", None).await?,
                    },
                    Err(e) => {
                        let err = json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}});
                        respond(&mut w, "400 Bad Request", Some(&err)).await?
                    }
                },
                // Sessions are not tracked: ending one is a no-op.
                "DELETE" => respond(&mut w, "200 OK", None).await?,
                // No server-to-client stream is offered.
                _ => respond(&mut w, "405 Method Not Allowed", None).await?,
            }
        }
        if close {
            break;
        }
    }
    Ok(())
}

/// Answers one JSON-RPC message; `None` for notifications and responses.
async fn handle(s: &Shared, msg: &Value) -> Option<Value> {
    let method = msg.get("method")?.as_str()?;
    let id = msg.get("id")?.clone();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or("");
            let version = if KNOWN_VERSIONS.contains(&asked) {
                asked
            } else {
                PROTOCOL_VERSION
            };
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": s.name, "version": env!("CARGO_PKG_VERSION") },
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => {
            let tools: Vec<Value> = s
                .host
                .tools()
                .into_iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": t.input_schema,
                        "annotations": { "readOnlyHint": t.read_only },
                    })
                })
                .collect();
            Ok(json!({ "tools": tools }))
        }
        "tools/call" => match params["name"].as_str() {
            Some(name) if s.host.tools().iter().any(|t| t.name == name) => {
                let args = match params.get("arguments") {
                    Some(a) if !a.is_null() => a.clone(),
                    _ => json!({}),
                };
                let r = s.host.call(name, args).await;
                Ok(json!({
                    "content": [{ "type": "text", "text": r.text }],
                    "isError": r.is_error,
                }))
            }
            Some(name) => Err((-32602, format!("unknown tool {name}"))),
            None => Err((-32602, "tools/call without a tool name".to_owned())),
        },
        other => Err((-32601, format!("method {other} not supported"))),
    };
    Some(match result {
        Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_and_origins() {
        assert!(token_ok(Some("Bearer abc"), "abc"));
        assert!(!token_ok(Some("Bearer abd"), "abc"));
        assert!(!token_ok(Some("abc"), "abc"));
        assert!(!token_ok(None, "abc"));
        assert!(origin_ok(None));
        assert!(origin_ok(Some("http://127.0.0.1:3000")));
        assert!(origin_ok(Some("http://localhost")));
        assert!(!origin_ok(Some("https://evil.example")));
        assert!(!origin_ok(Some("http://127.0.0.1.evil.example")));
    }
}
