//! MCP client: JSON-RPC 2.0 over stdio (newline-delimited) or Streamable HTTP (POST, JSON or SSE responses,
//! `Mcp-Session-Id`). Only the client side Agenium needs: initialize, tools/list, tools/call, ping.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;

/// Protocol revision Agenium asks for.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
/// Revisions whose tools/* shape this client understands.
pub const KNOWN_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("configuration: {0}")]
    Config(String),
    /// The connection is gone (process exited, stream closed, session expired): reconnect.
    #[error("connection lost: {0}")]
    Transport(String),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("server error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("no answer within {0:?}")]
    Timeout(Duration),
    /// The server wants a signed-in user (OAuth, ADR-97); the text says how.
    #[error("{0}")]
    AuthRequired(String),
    /// A sign-in step failed.
    #[error("sign-in: {0}")]
    Auth(String),
}

pub type Result<T> = std::result::Result<T, McpError>;

/// How to reach a server, with every placeholder already resolved.
#[derive(Clone)]
pub enum ConnectSpec {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: PathBuf,
    },
    Http {
        url: String,
        headers: BTreeMap<String, String>,
        /// The server's OAuth token, when it has no `Authorization` header of its own.
        bearer: Option<Arc<crate::oauth::Bearer>>,
    },
}

impl std::fmt::Debug for ConnectSpec {
    // Resolved values may be secrets: never print them.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectSpec::Stdio {
                command, args, env, ..
            } => f
                .debug_struct("Stdio")
                .field("command", command)
                .field("args", args)
                .field("env", &env.keys().collect::<Vec<_>>())
                .finish(),
            ConnectSpec::Http {
                url,
                headers,
                bearer,
            } => f
                .debug_struct("Http")
                .field("url", url)
                .field("headers", &headers.keys().collect::<Vec<_>>())
                .field("oauth", &bearer.is_some())
                .finish(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    /// `annotations.readOnlyHint`: safe to retry after a reconnect.
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallResult {
    pub text: String,
    pub is_error: bool,
}

#[async_trait]
trait Wire: Send + Sync {
    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value>;
    async fn notify(&self, method: &str, params: Value) -> Result<()>;
    fn alive(&self) -> bool;
    /// The negotiated revision (HTTP sends it as `MCP-Protocol-Version` on later requests).
    fn set_protocol(&self, _protocol: &str) {}
}

fn rpc_result(v: Value) -> Result<Value> {
    if let Some(e) = v.get("error") {
        return Err(McpError::Rpc {
            code: e["code"].as_i64().unwrap_or(0),
            message: e["message"].as_str().unwrap_or("").to_owned(),
        });
    }
    v.get("result")
        .cloned()
        .ok_or_else(|| McpError::Protocol("response without result".into()))
}

/// Answer to a request the server sends us (we offer no client capabilities except ping).
fn answer_server_request(msg: &Value) -> Value {
    if msg["method"] == "ping" {
        json!({ "jsonrpc": "2.0", "id": msg["id"], "result": {} })
    } else {
        json!({ "jsonrpc": "2.0", "id": msg["id"], "error": { "code": -32601, "message": "method not supported by this client" } })
    }
}

// ---------------------------------------------------------------- stdio

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>>;

struct Stdio {
    stdin: Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
    pending: Pending,
    next: AtomicI64,
    alive: Arc<AtomicBool>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    _child: Mutex<tokio::process::Child>,
    _guard: agenium_sandbox::TreeGuard,
}

async fn write_line(
    stdin: &tokio::sync::Mutex<tokio::process::ChildStdin>,
    v: &Value,
) -> Result<()> {
    let mut line = v.to_string();
    line.push('\n');
    let mut w = stdin.lock().await;
    w.write_all(line.as_bytes())
        .await
        .map_err(|e| McpError::Transport(e.to_string()))?;
    w.flush()
        .await
        .map_err(|e| McpError::Transport(e.to_string()))
}

impl Stdio {
    fn spawn(
        command: &str,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: &std::path::Path,
    ) -> Result<Stdio> {
        std::fs::create_dir_all(cwd)
            .map_err(|e| McpError::Config(format!("{}: {e}", cwd.display())))?;
        let program = agenium_sandbox::find_program(command)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| command.to_owned());
        let mut child = agenium_sandbox::service_command(&program, args, env, cwd)
            .spawn()
            .map_err(|e| McpError::Config(format!("cannot start {command:?}: {e}")))?;
        let guard = agenium_sandbox::TreeGuard::attach(&child);
        let stdin = Arc::new(tokio::sync::Mutex::new(
            child.stdin.take().expect("piped stdin"),
        ));
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr_pipe = child.stderr.take().expect("piped stderr");
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let stderr = Arc::new(Mutex::new(VecDeque::new()));

        let tail = stderr.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr_pipe).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let mut t = tail.lock().unwrap_or_else(|p| p.into_inner());
                if t.len() >= 20 {
                    t.pop_front();
                }
                t.push_back(l.chars().take(500).collect());
            }
        });

        let (p, a, w, tail) = (
            pending.clone(),
            alive.clone(),
            stdin.clone(),
            stderr.clone(),
        );
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let Ok(msg) = serde_json::from_str::<Value>(&l) else {
                    tracing::debug!(line = %l.chars().take(200).collect::<String>(), "MCP server wrote a non-JSON line");
                    continue;
                };
                if msg.get("method").is_some() {
                    if msg.get("id").is_some() {
                        let _ = write_line(&w, &answer_server_request(&msg)).await;
                    }
                    continue; // notifications are ignored
                }
                if let Some(id) = msg["id"].as_i64() {
                    if let Some(tx) = p.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
                        let _ = tx.send(Ok(msg));
                    }
                }
            }
            a.store(false, Ordering::SeqCst);
            let why = {
                let t = tail.lock().unwrap_or_else(|e| e.into_inner());
                let s: Vec<&str> = t.iter().map(String::as_str).collect();
                format!("the server process ended; last stderr: {}", s.join(" | "))
            };
            for (_, tx) in p.lock().unwrap_or_else(|e| e.into_inner()).drain() {
                let _ = tx.send(Err(McpError::Transport(why.clone())));
            }
        });
        Ok(Stdio {
            stdin,
            pending,
            next: AtomicI64::new(1),
            alive,
            stderr,
            _child: Mutex::new(child),
            _guard: guard,
        })
    }

    fn stderr_tail(&self) -> String {
        let t = self.stderr.lock().unwrap_or_else(|e| e.into_inner());
        t.iter().cloned().collect::<Vec<_>>().join(" | ")
    }
}

#[async_trait]
impl Wire for Stdio {
    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        if !self.alive() {
            return Err(McpError::Transport(format!(
                "the server process is not running; stderr: {}",
                self.stderr_tail()
            )));
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(e) = write_line(&self.stdin, &msg).await {
            self.pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(r)) => rpc_result(r?),
            Ok(Err(_)) => Err(McpError::Transport(
                "the server closed the connection".into(),
            )),
            Err(_) => {
                self.pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                let _ = self
                    .notify(
                        "notifications/cancelled",
                        json!({ "requestId": id, "reason": "timeout" }),
                    )
                    .await;
                Err(McpError::Timeout(timeout))
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        write_line(
            &self.stdin,
            &json!({ "jsonrpc": "2.0", "method": method, "params": params }),
        )
        .await
    }

    fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------- Streamable HTTP

struct Http {
    client: reqwest::Client,
    url: String,
    headers: reqwest::header::HeaderMap,
    session: Mutex<Option<String>>,
    protocol: Mutex<Option<String>>,
    next: AtomicI64,
    alive: AtomicBool,
    bearer: Option<Arc<crate::oauth::Bearer>>,
}

/// Splits complete SSE events out of `buf`, returning the `data` payload of each.
fn sse_events(buf: &mut String) -> Vec<String> {
    let mut out = Vec::new();
    loop {
        let norm = buf.replace("\r\n", "\n");
        let Some(end) = norm.find("\n\n") else {
            *buf = norm;
            return out;
        };
        let event = &norm[..end];
        let data: Vec<&str> = event
            .lines()
            .filter_map(|l| {
                l.strip_prefix("data:")
                    .map(|d| d.strip_prefix(' ').unwrap_or(d))
            })
            .collect();
        if !data.is_empty() {
            out.push(data.join("\n"));
        }
        *buf = norm[end + 2..].to_owned();
    }
}

impl Http {
    fn new(
        url: &str,
        headers: &BTreeMap<String, String>,
        bearer: Option<Arc<crate::oauth::Bearer>>,
    ) -> Result<Http> {
        let mut map = reqwest::header::HeaderMap::new();
        for (k, v) in headers {
            let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                .map_err(|_| McpError::Config(format!("invalid header name {k:?}")))?;
            let mut value = reqwest::header::HeaderValue::from_str(v)
                .map_err(|_| McpError::Config(format!("invalid value for header {k:?}")))?;
            value.set_sensitive(true);
            map.insert(name, value);
        }
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| McpError::Config(e.to_string()))?;
        Ok(Http {
            client,
            url: url.to_owned(),
            headers: map,
            session: Mutex::new(None),
            protocol: Mutex::new(None),
            next: AtomicI64::new(1),
            alive: AtomicBool::new(true),
            bearer,
        })
    }

    fn post(&self, body: &Value, token: Option<&str>) -> reqwest::RequestBuilder {
        let mut rb = self
            .client
            .post(&self.url)
            .headers(self.headers.clone())
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .body(body.to_string());
        if let Some(s) = self
            .session
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            rb = rb.header("Mcp-Session-Id", s);
        }
        if let Some(p) = self
            .protocol
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            rb = rb.header("MCP-Protocol-Version", p);
        }
        if let Some(t) = token {
            let mut v = reqwest::header::HeaderValue::from_str(&format!("Bearer {t}"))
                .unwrap_or_else(|_| reqwest::header::HeaderValue::from_static("Bearer -"));
            v.set_sensitive(true);
            rb = rb.header(reqwest::header::AUTHORIZATION, v);
        }
        rb
    }

    /// POSTs `body` with the OAuth token when the server has one: after a 401 the token is renewed once and the
    /// body sent again; a 401 or a 403 asking for more scope after that means the user must sign in.
    async fn send(&self, body: &Value) -> Result<reqwest::Response> {
        let Some(bearer) = &self.bearer else {
            return self
                .post(body, None)
                .send()
                .await
                .map_err(|e| McpError::Transport(e.to_string()));
        };
        let mut token = bearer.token().await;
        for attempt in 0..2 {
            let resp = self
                .post(body, token.as_deref())
                .send()
                .await
                .map_err(|e| McpError::Transport(e.to_string()))?;
            match resp.status().as_u16() {
                401 if attempt == 0 && bearer.recover(token.as_deref()).await => {
                    token = bearer.token().await;
                }
                401 => return Err(McpError::AuthRequired(bearer.sign_in_hint())),
                403 => {
                    let c = resp
                        .headers()
                        .get("www-authenticate")
                        .and_then(|v| v.to_str().ok())
                        .and_then(crate::oauth::parse_challenge);
                    if let Some(c) = c.filter(|c| c.error.as_deref() == Some("insufficient_scope"))
                    {
                        return Err(McpError::AuthRequired(format!(
                            "MCP server {} asks for more access ({}): sign in again with `agenium mcp login {} --scope …`",
                            bearer.server(),
                            c.scope.unwrap_or_default(),
                            bearer.server()
                        )));
                    }
                    return Ok(resp);
                }
                _ => return Ok(resp),
            }
        }
        Err(McpError::AuthRequired(bearer.sign_in_hint()))
    }

    async fn check(&self, resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status().as_u16();
        if status == 404
            && self
                .session
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some()
        {
            self.alive.store(false, Ordering::SeqCst);
            return Err(McpError::Transport("the server ended the session".into()));
        }
        if !(200..300).contains(&status) {
            let body: String = resp
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(500)
                .collect();
            return Err(McpError::Http { status, body });
        }
        Ok(resp)
    }

    async fn exchange(&self, id: i64, body: Value) -> Result<Value> {
        let resp = self.send(&body).await?;
        let resp = self.check(resp).await?;
        if let Some(s) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            *self.session.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.to_owned());
        }
        let ctype = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ctype.starts_with("text/event-stream") {
            let mut resp = resp;
            let mut buf = String::new();
            while let Some(chunk) = resp
                .chunk()
                .await
                .map_err(|e| McpError::Transport(e.to_string()))?
            {
                buf.push_str(&String::from_utf8_lossy(&chunk));
                for data in sse_events(&mut buf) {
                    let Ok(msg) = serde_json::from_str::<Value>(&data) else {
                        continue;
                    };
                    if msg.get("method").is_some() {
                        if msg.get("id").is_some() {
                            let _ = self.send(&answer_server_request(&msg)).await;
                        }
                        continue;
                    }
                    if msg["id"].as_i64() == Some(id) {
                        return rpc_result(msg);
                    }
                }
            }
            return Err(McpError::Transport(
                "the event stream ended without an answer".into(),
            ));
        }
        let text = resp
            .text()
            .await
            .map_err(|e| McpError::Transport(e.to_string()))?;
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| McpError::Protocol(format!("invalid JSON response: {e}")))?;
        let msg = match v {
            Value::Array(items) => items
                .into_iter()
                .find(|m| m["id"].as_i64() == Some(id))
                .ok_or_else(|| McpError::Protocol("batch without our response".into()))?,
            m => m,
        };
        rpc_result(msg)
    }
}

#[async_trait]
impl Wire for Http {
    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        tokio::time::timeout(timeout, self.exchange(id, body))
            .await
            .map_err(|_| McpError::Timeout(timeout))?
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let body = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        let resp = self.send(&body).await?;
        self.check(resp).await.map(|_| ())
    }

    fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    fn set_protocol(&self, protocol: &str) {
        *self.protocol.lock().unwrap_or_else(|e| e.into_inner()) = Some(protocol.to_owned());
    }
}

// ---------------------------------------------------------------- client

pub struct Client {
    wire: Box<dyn Wire>,
    pub protocol: String,
    pub server_info: Value,
    pub instructions: Option<String>,
}

/// Text of an MCP content list: text parts verbatim, other parts as short placeholders.
pub fn render_content(result: &Value) -> String {
    let mut parts = Vec::new();
    for c in result["content"].as_array().into_iter().flatten() {
        match c["type"].as_str() {
            Some("text") => parts.push(c["text"].as_str().unwrap_or("").to_owned()),
            Some("image") | Some("audio") => parts.push(format!(
                "[{} content omitted ({})]",
                c["type"].as_str().unwrap_or(""),
                c["mimeType"].as_str().unwrap_or("unknown type")
            )),
            Some("resource") => match c["resource"]["text"].as_str() {
                Some(t) => parts.push(t.to_owned()),
                None => parts.push(format!(
                    "[binary resource {}]",
                    c["resource"]["uri"].as_str().unwrap_or("")
                )),
            },
            Some("resource_link") => parts.push(format!(
                "[resource link {}]",
                c["uri"].as_str().unwrap_or("")
            )),
            _ => parts.push(c.to_string()),
        }
    }
    if parts.is_empty() {
        if let Some(s) = result.get("structuredContent") {
            return s.to_string();
        }
    }
    parts.join("\n")
}

impl Client {
    pub async fn connect(spec: &ConnectSpec, timeout: Duration) -> Result<Client> {
        let wire: Box<dyn Wire> = match spec {
            ConnectSpec::Stdio {
                command,
                args,
                env,
                cwd,
            } => Box::new(Stdio::spawn(command, args, env, cwd)?),
            ConnectSpec::Http {
                url,
                headers,
                bearer,
            } => Box::new(Http::new(url, headers, bearer.clone())?),
        };
        let init = wire
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "agenium", "version": env!("CARGO_PKG_VERSION") },
                }),
                timeout,
            )
            .await?;
        let protocol = init["protocolVersion"].as_str().unwrap_or("").to_owned();
        if !KNOWN_VERSIONS.contains(&protocol.as_str()) {
            // Newer revisions keep tools/list and tools/call compatible so far; note it and continue.
            tracing::warn!(%protocol, "MCP server uses a protocol revision this client does not know");
        }
        if init
            .get("capabilities")
            .and_then(|c| c.get("tools"))
            .is_none()
        {
            tracing::warn!("MCP server does not declare the tools capability");
        }
        wire.set_protocol(&protocol);
        let client = Client {
            wire,
            protocol,
            server_info: init["serverInfo"].clone(),
            instructions: init["instructions"].as_str().map(str::to_owned),
        };
        client
            .wire
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(client)
    }

    pub fn alive(&self) -> bool {
        self.wire.alive()
    }

    pub async fn ping(&self, timeout: Duration) -> Result<()> {
        self.wire
            .request("ping", json!({}), timeout)
            .await
            .map(|_| ())
    }

    pub async fn list_tools(&self, timeout: Duration) -> Result<Vec<ToolInfo>> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..50 {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let r = self.wire.request("tools/list", params, timeout).await?;
            for t in r["tools"].as_array().into_iter().flatten() {
                let Some(name) = t["name"].as_str() else {
                    continue;
                };
                let mut schema = t["inputSchema"].clone();
                if !schema.is_object() {
                    schema = json!({ "type": "object", "properties": {} });
                }
                out.push(ToolInfo {
                    name: name.to_owned(),
                    description: t["description"]
                        .as_str()
                        .or(t["title"].as_str())
                        .unwrap_or("")
                        .to_owned(),
                    input_schema: schema,
                    read_only: t["annotations"]["readOnlyHint"].as_bool().unwrap_or(false),
                });
            }
            match r["nextCursor"].as_str() {
                Some(c) if !c.is_empty() => cursor = Some(c.to_owned()),
                _ => return Ok(out),
            }
        }
        Err(McpError::Protocol(
            "tools/list pagination does not end".into(),
        ))
    }

    pub async fn call_tool(
        &self,
        name: &str,
        args: Value,
        timeout: Duration,
    ) -> Result<CallResult> {
        let args = if args.is_null() { json!({}) } else { args };
        let r = self
            .wire
            .request(
                "tools/call",
                json!({ "name": name, "arguments": args }),
                timeout,
            )
            .await?;
        Ok(CallResult {
            text: render_content(&r),
            is_error: r["isError"].as_bool().unwrap_or(false),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_splitting() {
        let mut b =
            "event: message\ndata: {\"a\":1}\n\ndata: x\r\ndata: y\r\n\r\ndata: part".to_string();
        assert_eq!(
            sse_events(&mut b),
            vec!["{\"a\":1}".to_string(), "x\ny".into()]
        );
        assert_eq!(b, "data: part");
    }

    #[test]
    fn content_rendering() {
        let r = json!({"content": [
            {"type": "text", "text": "hello"},
            {"type": "image", "data": "...", "mimeType": "image/png"},
            {"type": "resource", "resource": {"uri": "file:///a", "text": "inside"}},
            {"type": "resource_link", "uri": "file:///b", "name": "b"}
        ]});
        assert_eq!(
            render_content(&r),
            "hello\n[image content omitted (image/png)]\ninside\n[resource link file:///b]"
        );
        assert_eq!(
            render_content(&json!({"content": [], "structuredContent": {"n": 1}})),
            "{\"n\":1}"
        );
    }
}
