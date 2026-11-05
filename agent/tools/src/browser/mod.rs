//! A headless browser of the run (ADR-158): Edge or Chrome as found on the machine, started on the first browser tool
//! call with a fresh profile in the run's temp folder, driven over the DevTools protocol, ended with the run (the tool
//! context is dropped). It runs outside the sandbox, as MCP servers do, and reaches only the machine itself and the
//! project's network allowlist: every other host goes to a closed proxy port and fails.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::ToolError;

mod find;
pub use find::find_browser;

/// How long the browser has to start and say where its DevTools listen.
const START: Duration = Duration::from_secs(20);
/// How long one protocol call may take (a page load waits in steps of it).
const CALL: Duration = Duration::from_secs(30);

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The run's browser, started on first use.
#[derive(Default, Clone)]
pub struct Session(Arc<Mutex<Option<Browser>>>);

pub struct Browser {
    _child: tokio::process::Child,
    _tree: agenium_sandbox::TreeGuard,
    ws: Ws,
    next: u64,
    /// Console errors and uncaught exceptions of the page since the last tool call read them.
    pub errors: Vec<String>,
    _profile: tempfile::TempDir,
}

impl Session {
    /// The browser, started with `allow` (hosts beside the machine itself the pages may reach) when it is not yet.
    pub async fn get(
        &self,
        temp: &Path,
        allow: &[String],
    ) -> Result<tokio::sync::MappedMutexGuard<'_, Browser>, ToolError> {
        let mut g = self.0.lock().await;
        if g.is_none() {
            *g = Some(Browser::start(temp, allow).await?);
        }
        Ok(tokio::sync::MutexGuard::map(g, |b| b.as_mut().unwrap()))
    }
}

fn err(code: &str, m: impl std::fmt::Display) -> ToolError {
    ToolError::new(code, m.to_string())
}

impl Browser {
    async fn start(temp: &Path, allow: &[String]) -> Result<Browser, ToolError> {
        let exe = find_browser().ok_or_else(|| {
            err(
                "no_browser",
                "no Edge or Chrome found on this machine (set AGENIUM_BROWSER to a Chromium browser's program)",
            )
        })?;
        std::fs::create_dir_all(temp).map_err(|e| err("browser", e))?;
        let profile = tempfile::Builder::new()
            .prefix("browser-")
            .tempdir_in(temp)
            .map_err(|e| err("browser", e))?;
        // Every host but the machine itself and the allowlist goes to a closed port (port 9, discard): its request fails.
        let mut bypass = vec![
            "localhost".to_owned(),
            "127.0.0.1".to_owned(),
            "[::1]".to_owned(),
            "*.localhost".to_owned(),
        ];
        bypass.extend(
            allow
                .iter()
                .map(|a| a.trim().to_owned())
                .filter(|a| !a.is_empty()),
        );
        let mut cmd = tokio::process::Command::new(&exe);
        cmd.args([
            "--headless=new",
            "--remote-debugging-port=0",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-sync",
            "--disable-background-networking",
            "--disable-component-update",
            "--mute-audio",
            "--window-size=1280,800",
            "--proxy-server=http://127.0.0.1:9",
        ])
        .arg(format!("--proxy-bypass-list={}", bypass.join(";")))
        .arg(format!("--user-data-dir={}", profile.path().display()))
        .arg("about:blank")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd
            .spawn()
            .map_err(|e| err("browser", format!("{}: {e}", exe.display())))?;
        let tree = agenium_sandbox::TreeGuard::attach(&child);
        // The browser says where its DevTools listen on stderr: "DevTools listening on ws://127.0.0.1:<port>/...".
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| err("browser", "no stderr"))?;
        let mut lines = BufReader::new(stderr).lines();
        let port = tokio::time::timeout(START, async {
            while let Ok(Some(l)) = lines.next_line().await {
                if let Some(rest) = l.split("ws://127.0.0.1:").nth(1) {
                    return rest.split('/').next().and_then(|p| p.parse::<u16>().ok());
                }
            }
            None
        })
        .await
        .ok()
        .flatten()
        .ok_or_else(|| {
            err(
                "browser",
                format!("{} did not start its DevTools", exe.display()),
            )
        })?;
        // Its later output is not read: drain it so the browser never blocks on a full pipe.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        let page = page_socket(port).await?;
        let (ws, _) = tokio_tungstenite::connect_async(page.as_str())
            .await
            .map_err(|e| err("browser", format!("DevTools: {e}")))?;
        let mut b = Browser {
            _child: child,
            _tree: tree,
            ws,
            next: 0,
            errors: vec![],
            _profile: profile,
        };
        b.call("Runtime.enable", json!({})).await?;
        b.call("Page.enable", json!({})).await?;
        Ok(b)
    }

    /// One protocol call on the page; events met while waiting are kept (console errors, exceptions).
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, ToolError> {
        self.next += 1;
        let id = self.next;
        let msg = json!({ "id": id, "method": method, "params": params }).to_string();
        self.ws
            .send(WsMessage::text(msg))
            .await
            .map_err(|e| err("browser", e))?;
        let deadline = tokio::time::Instant::now() + CALL;
        loop {
            let m = tokio::time::timeout_at(deadline, self.ws.next())
                .await
                .map_err(|_| {
                    err(
                        "browser_timeout",
                        format!("{method} took longer than {}s", CALL.as_secs()),
                    )
                })?
                .ok_or_else(|| err("browser", "the browser closed"))?
                .map_err(|e| err("browser", e))?;
            let WsMessage::Text(t) = m else { continue };
            let v: Value = serde_json::from_str(t.as_str()).unwrap_or(Value::Null);
            if v["id"] == id {
                if let Some(e) = v.get("error") {
                    return Err(err(
                        "browser",
                        format!("{method}: {}", e["message"].as_str().unwrap_or("failed")),
                    ));
                }
                return Ok(v["result"].clone());
            }
            self.keep_event(&v);
        }
    }

    fn keep_event(&mut self, v: &Value) {
        let p = &v["params"];
        let text = match v["method"].as_str() {
            Some("Runtime.exceptionThrown") => p["exceptionDetails"]["exception"]["description"]
                .as_str()
                .or(p["exceptionDetails"]["text"].as_str())
                .map(str::to_owned),
            Some("Runtime.consoleAPICalled") if p["type"] == "error" => Some(
                p["args"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|a| {
                        a["value"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| a["description"].as_str().unwrap_or("").to_owned())
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        };
        if let Some(t) = text.filter(|t| !t.is_empty()) {
            if self.errors.len() < 20 {
                self.errors.push(t.chars().take(300).collect());
            }
        }
    }

    /// Evaluates `expression` in the page and returns its value (awaiting a promise).
    pub async fn eval(&mut self, expression: &str) -> Result<Value, ToolError> {
        let r = self
            .call(
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
            )
            .await?;
        if let Some(e) = r.get("exceptionDetails") {
            let d = e["exception"]["description"]
                .as_str()
                .or(e["text"].as_str())
                .unwrap_or("failed");
            return Err(err("script_error", d.chars().take(500).collect::<String>()));
        }
        Ok(r["result"]["value"].clone())
    }

    /// Waits until the page has loaded (or `CALL` passed), then a little longer for what it renders after.
    pub async fn settle(&mut self) -> Result<(), ToolError> {
        let deadline = tokio::time::Instant::now() + CALL;
        while tokio::time::Instant::now() < deadline {
            if self.eval("document.readyState").await? == "complete" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        Ok(())
    }
}

impl Drop for Browser {
    // The profile folder is removed after the fields drop; on Windows it stays locked until the browser has exited.
    fn drop(&mut self) {
        let _ = self._child.start_kill();
        for _ in 0..30 {
            if matches!(self._child.try_wait(), Ok(Some(_)) | Err(_)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// The DevTools socket of the browser's first page.
async fn page_socket(port: u16) -> Result<String, ToolError> {
    let http = reqwest::Client::new();
    for _ in 0..50 {
        if let Ok(r) = http
            .get(format!("http://127.0.0.1:{port}/json/list"))
            .send()
            .await
        {
            if let Ok(v) = r
                .text()
                .await
                .map(|t| serde_json::from_str::<Value>(&t).unwrap_or(Value::Null))
            {
                if let Some(ws) = v
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|t| t["type"] == "page")
                    .and_then(|t| t["webSocketDebuggerUrl"].as_str())
                {
                    return Ok(ws.to_owned());
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(err("browser", "the browser has no page"))
}

/// Whether a page at `url` may be opened: the machine itself, a host of the allowlist, or a file of `root`.
pub fn url_allowed(url: &str, allow: &[String], root: &Path) -> Result<String, ToolError> {
    let u =
        reqwest::Url::parse(url.trim()).map_err(|e| err("invalid_url", format!("{url}: {e}")))?;
    match u.scheme() {
        "http" | "https" => {
            let host = u
                .host_str()
                .unwrap_or_default()
                .trim_matches(['[', ']'])
                .to_ascii_lowercase();
            let local = host == "localhost"
                || host.ends_with(".localhost")
                || host == "127.0.0.1"
                || host == "::1";
            if local || crate::work::domain_allowed(&host, allow) {
                Ok(u.to_string())
            } else {
                Err(err(
                    "domain_not_allowed",
                    format!("{host} is not this machine nor in the network allowlist; use request_permission (kind=network)"),
                ))
            }
        }
        "file" => {
            let p: PathBuf = u.to_file_path().map_err(|_| err("invalid_url", url))?;
            if crate::paths::confined(&p, root) {
                Ok(u.to_string())
            } else {
                Err(err(
                    "outside_worktree",
                    format!("{} is not a file of your worktree", p.display()),
                ))
            }
        }
        s => Err(err(
            "invalid_url",
            format!("{s}: only http, https and files of your worktree open"),
        )),
    }
}
