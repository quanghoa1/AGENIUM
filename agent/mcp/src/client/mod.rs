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

mod stdio;

#[cfg(test)]
use http::sse_events;
use http::{push_utf8, Http};
use sse::Sse;
use stdio::Stdio;
mod http;
mod sse;

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

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>>;

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
