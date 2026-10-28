//! The legacy HTTP+SSE transport: one event stream down, POSTs to the endpoint it names.

use super::*;

/// Complete SSE events out of `buf`: (event name, data).
pub(super) fn sse_frames(buf: &mut String) -> Vec<(String, String)> {
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

pub(super) struct SseShared {
    client: reqwest::Client,
    headers: reqwest::header::HeaderMap,
    post_url: Mutex<Option<String>>,
    pending: Pending,
    alive: AtomicBool,
}

impl SseShared {
    pub(super) async fn post(&self, body: &Value) -> Result<()> {
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

pub(super) struct Sse {
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
    pub(super) async fn connect(
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
