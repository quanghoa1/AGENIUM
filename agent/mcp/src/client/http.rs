//! The streamable HTTP transport (one POST per message, JSON or an SSE stream back), with its OAuth token.

use super::*;

pub(super) struct Http {
    pub(super) client: reqwest::Client,
    pub(super) url: String,
    pub(super) headers: reqwest::header::HeaderMap,
    pub(super) session: Mutex<Option<String>>,
    pub(super) protocol: Mutex<Option<String>>,
    pub(super) next: AtomicI64,
    pub(super) alive: AtomicBool,
    pub(super) bearer: Option<Arc<crate::oauth::Bearer>>,
}

/// Appends `chunk` to `buf` as text. A character split between two network chunks is kept in `partial` until its
/// last byte arrives (decoding each chunk alone would turn it into U+FFFD).
pub(super) fn push_utf8(buf: &mut String, partial: &mut Vec<u8>, chunk: &[u8]) {
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
pub(super) fn sse_events(buf: &mut String) -> Vec<String> {
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
    pub(super) fn new(
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

    pub(super) fn post(&self, body: &Value, token: Option<&str>) -> reqwest::RequestBuilder {
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
    pub(super) async fn send(&self, body: &Value) -> Result<reqwest::Response> {
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

    pub(super) async fn check(&self, resp: reqwest::Response) -> Result<reqwest::Response> {
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

    pub(super) async fn exchange(&self, id: i64, body: Value) -> Result<Value> {
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
