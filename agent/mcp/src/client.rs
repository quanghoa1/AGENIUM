//! MCP client: JSON-RPC 2.0 over stdio (newline-delimited), Streamable HTTP (POST, JSON or SSE responses,
//! `Mcp-Session-Id`), or the legacy HTTP+SSE transport of revision 2024-11-05 (a GET event stream that names the POST
//! endpoint and carries the answers), used when a server refuses Streamable HTTP. Only the client side Agenium needs:
//! initialize, tools, resources, prompts, ping.

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

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CallResult {
    pub text: String,
    pub is_error: bool,
    /// The result's images (a screenshot): `(media type, base64)`.
    pub images: Vec<(String, String)>,
}

/// Image types a model takes.
const IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// The images of an MCP content list that a model can look at.
pub fn content_images(result: &Value) -> Vec<(String, String)> {
    result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["type"] == "image")
        .filter_map(|c| {
            let t = c["mimeType"].as_str()?;
            let d = c["data"].as_str()?;
            IMAGE_TYPES
                .contains(&t)
                .then(|| (t.to_owned(), d.to_owned()))
        })
        .collect()
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

/// Appends `chunk` to `buf` as text. A character split between two network chunks is kept in `partial` until its
/// last byte arrives (decoding each chunk alone would turn it into U+FFFD).
fn push_utf8(buf: &mut String, partial: &mut Vec<u8>, chunk: &[u8]) {
    partial.extend_from_slice(chunk);
    let valid = match std::str::from_utf8(partial) {
        Ok(s) => s.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        // An invalid sequence (not just an unfinished one): keep what can be read, replaced as usual.
        Err(_) => partial.len(),
    };
    buf.push_str(&String::from_utf8_lossy(&partial[..valid]));
    partial.drain(..valid);
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
        // Redirects stay on the server's origin: its configured headers (API keys) go with every request, and reqwest
        // strips only `Authorization` and cookies when a redirect leaves the host.
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::custom(|a| {
                if !a.previous().first().is_some_and(|f| f.origin() == a.url().origin()) {
                    a.error("the MCP server redirected to another origin; its headers are not sent there")
                } else if a.previous().len() > 5 {
                    a.error("too many redirects")
                } else {
                    a.follow()
                }
            }))
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
            let mut partial = Vec::new();
            while let Some(chunk) = resp
                .chunk()
                .await
                .map_err(|e| McpError::Transport(e.to_string()))?
            {
                push_utf8(&mut buf, &mut partial, &chunk);
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

// ---------------------------------------------------------------- legacy HTTP+SSE (revision 2024-11-05)

/// Complete SSE events out of `buf`: (event name, data).
fn sse_frames(buf: &mut String) -> Vec<(String, String)> {
    let mut out = Vec::new();
    loop {
        let norm = buf.replace("\r\n", "\n");
        let Some(end) = norm.find("\n\n") else {
            *buf = norm;
            return out;
        };
        let event = &norm[..end];
        let mut name = String::from("message");
        let mut data = Vec::new();
        for l in event.lines() {
            if let Some(v) = l.strip_prefix("event:") {
                name = v.trim().to_owned();
            } else if let Some(v) = l.strip_prefix("data:") {
                data.push(v.strip_prefix(' ').unwrap_or(v).to_owned());
            }
        }
        if !data.is_empty() {
            out.push((name, data.join("\n")));
        }
        *buf = norm[end + 2..].to_owned();
    }
}

struct SseShared {
    client: reqwest::Client,
    headers: reqwest::header::HeaderMap,
    post_url: Mutex<Option<String>>,
    pending: Pending,
    alive: AtomicBool,
}

impl SseShared {
    async fn post(&self, body: &Value) -> Result<()> {
        let url = self
            .post_url
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| McpError::Transport("no endpoint yet".into()))?;
        let resp = self
            .client
            .post(url)
            .headers(self.headers.clone())
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| McpError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
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
        Ok(())
    }
}

struct Sse {
    shared: Arc<SseShared>,
    next: AtomicI64,
    reader: tokio::task::JoinHandle<()>,
}

impl Drop for Sse {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

impl Sse {
    /// Opens the event stream and waits for the `endpoint` event that names where requests go.
    async fn connect(
        url: &str,
        headers: &BTreeMap<String, String>,
        token: Option<String>,
        timeout: Duration,
    ) -> Result<Sse> {
        let http = Http::new(url, headers, None)?;
        let mut map = http.headers.clone();
        if let Some(t) = token {
            let mut v = reqwest::header::HeaderValue::from_str(&format!("Bearer {t}"))
                .map_err(|_| McpError::Config("invalid token".into()))?;
            v.set_sensitive(true);
            map.insert(reqwest::header::AUTHORIZATION, v);
        }
        let resp = http
            .client
            .get(url)
            .headers(map.clone())
            .header("Accept", "text/event-stream")
            .send()
            .await
            .map_err(|e| McpError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(McpError::Http {
                status,
                body: "the legacy SSE endpoint refused the stream".into(),
            });
        }
        let shared = Arc::new(SseShared {
            client: http.client.clone(),
            headers: map,
            post_url: Mutex::new(None),
            pending: Arc::new(Mutex::new(HashMap::new())),
            alive: AtomicBool::new(true),
        });
        let (ready_tx, ready_rx) = oneshot::channel::<String>();
        let base = reqwest::Url::parse(url).map_err(|e| McpError::Config(e.to_string()))?;
        let sh = shared.clone();
        let stream_base = base.clone();
        let reader = tokio::spawn(async move {
            let mut resp = resp;
            let mut ready = Some(ready_tx);
            let mut buf = String::new();
            let mut partial = Vec::new();
            while let Ok(Some(chunk)) = resp.chunk().await {
                push_utf8(&mut buf, &mut partial, &chunk);
                for (event, data) in sse_frames(&mut buf) {
                    if event == "endpoint" {
                        if let (Ok(u), Some(tx)) = (stream_base.join(data.trim()), ready.take()) {
                            let _ = tx.send(u.to_string());
                        }
                        continue;
                    }
                    let Ok(msg) = serde_json::from_str::<Value>(&data) else {
                        continue;
                    };
                    if msg.get("method").is_some() {
                        if msg.get("id").is_some() {
                            let _ = sh.post(&answer_server_request(&msg)).await;
                        }
                        continue;
                    }
                    if let Some(id) = msg["id"].as_i64() {
                        let tx = sh
                            .pending
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .remove(&id);
                        if let Some(tx) = tx {
                            let _ = tx.send(rpc_result(msg));
                        }
                    }
                }
            }
            sh.alive.store(false, Ordering::SeqCst);
            for (_, tx) in sh.pending.lock().unwrap_or_else(|e| e.into_inner()).drain() {
                let _ = tx.send(Err(McpError::Transport("the event stream ended".into())));
            }
        });
        let endpoint = tokio::time::timeout(timeout, ready_rx)
            .await
            .map_err(|_| McpError::Timeout(timeout))?
            .map_err(|_| {
                McpError::Transport("the event stream ended before naming its endpoint".into())
            })?;
        // The endpoint must stay on the server's origin.
        let e = reqwest::Url::parse(&endpoint).map_err(|e| McpError::Protocol(e.to_string()))?;
        if e.origin() != base.origin() {
            reader.abort();
            return Err(McpError::Protocol(format!(
                "the server named an endpoint on another origin ({endpoint})"
            )));
        }
        *shared.post_url.lock().unwrap_or_else(|e| e.into_inner()) = Some(endpoint);
        Ok(Sse {
            shared,
            next: AtomicI64::new(1),
            reader,
        })
    }
}

#[async_trait]
impl Wire for Sse {
    async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.shared
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(e) = self.shared.post(&body).await {
            self.shared
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(McpError::Transport("the event stream ended".into())),
            Err(_) => {
                self.shared
                    .pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                Err(McpError::Timeout(timeout))
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.shared
            .post(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    fn alive(&self) -> bool {
        self.shared.alive.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------- client

pub struct Client {
    wire: Box<dyn Wire>,
    pub protocol: String,
    pub server_info: Value,
    pub instructions: Option<String>,
    /// The server's declared capabilities (`tools`, `resources`, `prompts`, ...).
    pub capabilities: Value,
}

/// Text of an MCP content list: text parts verbatim, other parts as short placeholders.
pub fn render_content(result: &Value) -> String {
    let mut parts = Vec::new();
    for c in result["content"].as_array().into_iter().flatten() {
        match c["type"].as_str() {
            Some("text") => parts.push(c["text"].as_str().unwrap_or("").to_owned()),
            Some("image") if IMAGE_TYPES.contains(&c["mimeType"].as_str().unwrap_or("")) => parts
                .push(format!(
                    "[image ({}): shown after the tool results]",
                    c["mimeType"].as_str().unwrap_or("")
                )),
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
        let hello = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "agenium", "version": env!("CARGO_PKG_VERSION") },
        });
        let (wire, init): (Box<dyn Wire>, Value) = match spec {
            ConnectSpec::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                let w: Box<dyn Wire> = Box::new(Stdio::spawn(command, args, env, cwd)?);
                let init = w.request("initialize", hello, timeout).await?;
                (w, init)
            }
            ConnectSpec::Http {
                url,
                headers,
                bearer,
            } => {
                let w: Box<dyn Wire> = Box::new(Http::new(url, headers, bearer.clone())?);
                match w.request("initialize", hello.clone(), timeout).await {
                    Ok(init) => (w, init),
                    // A server of revision 2024-11-05 refuses the POST: its URL is the legacy event stream.
                    Err(McpError::Http {
                        status: 400 | 404 | 405,
                        ..
                    }) => {
                        let token = match bearer {
                            Some(b) => b.token().await,
                            None => None,
                        };
                        let s: Box<dyn Wire> =
                            Box::new(Sse::connect(url, headers, token, timeout).await?);
                        let init = s.request("initialize", hello, timeout).await?;
                        tracing::info!(%url, "MCP server uses the legacy HTTP+SSE transport");
                        (s, init)
                    }
                    Err(e) => return Err(e),
                }
            }
        };
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
            capabilities: init["capabilities"].clone(),
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

    /// Every page of a paginated list method, `key` naming the items (`resources`, `prompts`).
    async fn list_all(&self, method: &str, key: &str, timeout: Duration) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..50 {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let r = self.wire.request(method, params, timeout).await?;
            out.extend(r[key].as_array().into_iter().flatten().cloned());
            match r["nextCursor"].as_str() {
                Some(c) if !c.is_empty() => cursor = Some(c.to_owned()),
                _ => return Ok(out),
            }
        }
        Err(McpError::Protocol(format!(
            "{method} pagination does not end"
        )))
    }

    /// `resources/list` as text: one line per resource (`uri — name: description [mimeType]`).
    pub async fn list_resources(&self, timeout: Duration) -> Result<String> {
        let items = self
            .list_all("resources/list", "resources", timeout)
            .await?;
        Ok(items
            .iter()
            .map(|r| {
                let mut line = format!(
                    "{} — {}",
                    r["uri"].as_str().unwrap_or("?"),
                    r["name"].as_str().or(r["title"].as_str()).unwrap_or("")
                );
                if let Some(d) = r["description"].as_str() {
                    line.push_str(&format!(": {d}"));
                }
                if let Some(m) = r["mimeType"].as_str() {
                    line.push_str(&format!(" [{m}]"));
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// `resources/read`: text contents verbatim, binary ones as a placeholder.
    pub async fn read_resource(&self, uri: &str, timeout: Duration) -> Result<String> {
        let r = self
            .wire
            .request("resources/read", json!({ "uri": uri }), timeout)
            .await?;
        Ok(r["contents"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| match c["text"].as_str() {
                Some(t) => t.to_owned(),
                None => format!(
                    "[binary content of {} ({})]",
                    c["uri"].as_str().unwrap_or(uri),
                    c["mimeType"].as_str().unwrap_or("unknown type")
                ),
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// `prompts/list` as text: one line per prompt with its arguments.
    pub async fn list_prompts(&self, timeout: Duration) -> Result<String> {
        let items = self.list_all("prompts/list", "prompts", timeout).await?;
        Ok(items
            .iter()
            .map(|p| {
                let args: Vec<String> = p["arguments"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|a| {
                        format!(
                            "{}{}",
                            a["name"].as_str().unwrap_or("?"),
                            if a["required"] == true { "" } else { "?" }
                        )
                    })
                    .collect();
                format!(
                    "{}({}){}",
                    p["name"].as_str().unwrap_or("?"),
                    args.join(", "),
                    p["description"]
                        .as_str()
                        .map(|d| format!(": {d}"))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// `prompts/get`: the prompt's messages as text (`[role] text`).
    pub async fn get_prompt(&self, name: &str, args: Value, timeout: Duration) -> Result<String> {
        let args = if args.is_object() { args } else { json!({}) };
        let r = self
            .wire
            .request(
                "prompts/get",
                json!({ "name": name, "arguments": args }),
                timeout,
            )
            .await?;
        Ok(r["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| {
                let text = match m["content"]["type"].as_str() {
                    Some("text") => m["content"]["text"].as_str().unwrap_or("").to_owned(),
                    _ => render_content(&json!({ "content": [m["content"].clone()] })),
                };
                format!("[{}] {text}", m["role"].as_str().unwrap_or("user"))
            })
            .collect::<Vec<_>>()
            .join("\n"))
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
            images: content_images(&r),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_character_split_between_chunks_survives() {
        let text = "data: Xin chào thế giới

"
        .as_bytes();
        let (mut buf, mut partial) = (String::new(), Vec::new());
        // Split inside the two-byte "à".
        let cut = text.iter().position(|&b| b == 0xC3).unwrap() + 1;
        push_utf8(&mut buf, &mut partial, &text[..cut]);
        push_utf8(&mut buf, &mut partial, &text[cut..]);
        assert_eq!(sse_events(&mut buf), ["Xin chào thế giới"]);
        assert!(partial.is_empty());
    }

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

    /// Review of 2026-10-27: the Streamable HTTP client followed a redirect to another host with the server's
    /// configured headers (`X-API-Key`). A redirect within the origin is still followed.
    #[tokio::test]
    async fn headers_never_follow_a_redirect_to_another_origin() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        async fn serve(
            reply: impl Fn(&str) -> String + Send + Sync + 'static,
        ) -> (u16, Arc<Mutex<Vec<String>>>) {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = l.local_addr().unwrap().port();
            let seen = Arc::new(Mutex::new(Vec::new()));
            let s2 = seen.clone();
            tokio::spawn(async move {
                while let Ok((mut s, _)) = l.accept().await {
                    // The head, then the body by its length.
                    let mut b = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        let n = s.read(&mut chunk).await.unwrap_or(0);
                        b.extend_from_slice(&chunk[..n]);
                        let text = String::from_utf8_lossy(&b).to_string();
                        let done = text.split_once("\r\n\r\n").is_some_and(|(head, body)| {
                            let len = head
                                .lines()
                                .find_map(|l| {
                                    l.to_ascii_lowercase()
                                        .strip_prefix("content-length:")
                                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                                })
                                .unwrap_or(0);
                            body.len() >= len
                        });
                        if n == 0 || done {
                            break;
                        }
                    }
                    let req = String::from_utf8_lossy(&b).to_string();
                    s2.lock().unwrap().push(req.clone());
                    let _ = s.write_all(reply(&req).as_bytes()).await;
                }
            });
            (port, seen)
        }
        let init = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"t"}}}"#;
        let ok = move |req: &str| {
            let body = if req.contains("\"initialize\"") {
                init
            } else {
                ""
            };
            let status = if body.is_empty() {
                "202 Accepted"
            } else {
                "200 OK"
            };
            format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
        };
        let (other, other_seen) = serve(ok).await;
        let (port, _) = serve(move |req: &str| {
            if req.starts_with("POST /moved") {
                ok(req)
            } else if req.starts_with("POST /local") {
                "HTTP/1.1 307 Temporary Redirect\r\nlocation: /moved\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".into()
            } else {
                // Another port is another origin.
                format!("HTTP/1.1 307 Temporary Redirect\r\nlocation: http://127.0.0.1:{other}/mcp\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            }
        })
        .await;
        let spec = |path: &str| ConnectSpec::Http {
            url: format!("http://127.0.0.1:{port}{path}"),
            headers: [("X-API-Key".to_owned(), "k-secret".to_owned())].into(),
            bearer: None,
        };
        assert!(Client::connect(&spec("/mcp"), Duration::from_secs(5))
            .await
            .is_err());
        assert!(
            other_seen.lock().unwrap().is_empty(),
            "the other origin got {:?}",
            other_seen.lock().unwrap()
        );
        assert!(Client::connect(&spec("/local"), Duration::from_secs(5))
            .await
            .is_ok());
    }

    #[test]
    fn content_rendering() {
        let r = json!({"content": [
            {"type": "text", "text": "hello"},
            {"type": "image", "data": "iVBO", "mimeType": "image/png"},
            {"type": "image", "data": "...", "mimeType": "image/tiff"},
            {"type": "resource", "resource": {"uri": "file:///a", "text": "inside"}},
            {"type": "resource_link", "uri": "file:///b", "name": "b"}
        ]});
        assert_eq!(
            render_content(&r),
            "hello\n[image (image/png): shown after the tool results]\n[image content omitted (image/tiff)]\ninside\n[resource link file:///b]"
        );
        // Only the types a model takes are handed on.
        assert_eq!(
            content_images(&r),
            vec![("image/png".to_owned(), "iVBO".to_owned())]
        );
        assert_eq!(
            render_content(&json!({"content": [], "structuredContent": {"n": 1}})),
            "{\"n\":1}"
        );
    }
}
