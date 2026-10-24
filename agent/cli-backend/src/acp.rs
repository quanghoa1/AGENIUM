//! Agent Client Protocol (ACP, protocol version 1) client: JSON-RPC 2.0, one message per line, over the stdio of a
//! CLI started as an ACP server (`devin acp`, `opencode acp`). Agenium is the *client*: it creates the session,
//! sends prompts, receives `session/update` notifications and answers the agent's `session/request_permission`.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agenium_sandbox::TreeGuard;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{mpsc, oneshot};

pub const PROTOCOL_VERSION: u64 = 1;
/// Bytes of the CLI's stderr kept for error messages.
const STDERR_TAIL: usize = 16 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("cannot start {program}: {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },
    #[error("the CLI exited or closed its output{0}")]
    Closed(String),
    #[error("agent error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("no answer to {method} within {timeout:?}")]
    Timeout { method: String, timeout: Duration },
    #[error("protocol: {0}")]
    Protocol(String),
}

/// A message the agent sent on its own.
#[derive(Debug, Clone)]
pub enum Incoming {
    Notification {
        method: String,
        params: Value,
    },
    /// Must be answered with [`Conn::respond`] (ACP agents wait for the answer).
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

type Pending = Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>;

/// One running CLI in ACP mode. Dropping it kills the CLI's whole process tree.
pub struct Conn {
    stdin: tokio::sync::Mutex<ChildStdin>,
    pending: Pending,
    next: AtomicI64,
    stderr: Arc<Mutex<VecDeque<u8>>>,
    child: Mutex<Option<Child>>,
    _tree: TreeGuard,
}

impl Conn {
    /// Starts `program args` in `cwd` with a clean environment plus `env`. Returns the connection and the stream
    /// of agent-initiated messages (it ends when the CLI exits).
    pub fn spawn(
        program: &Path,
        args: &[String],
        env: &BTreeMap<String, String>,
        cwd: &Path,
    ) -> Result<(Conn, mpsc::UnboundedReceiver<Incoming>), AcpError> {
        let name = program.display().to_string();
        let mut child = agenium_sandbox::service_command(&name, args, env, cwd)
            .spawn()
            .map_err(|source| AcpError::Spawn {
                program: name.clone(),
                source,
            })?;
        let tree = TreeGuard::attach(&child);
        let (Some(stdin), Some(stdout), Some(mut stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(AcpError::Protocol("stdio pipes missing".into()));
        };
        let pending: Pending = Arc::default();
        let tail: Arc<Mutex<VecDeque<u8>>> = Arc::default();
        let (tx, rx) = mpsc::unbounded_channel();
        {
            let pending = pending.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else {
                        tracing::debug!(line = %line.chars().take(200).collect::<String>(), "non-JSON line from ACP agent");
                        continue;
                    };
                    let method = msg.get("method").and_then(Value::as_str).map(str::to_owned);
                    match (method, msg.get("id")) {
                        (Some(method), Some(id)) if !id.is_null() => {
                            let _ = tx.send(Incoming::Request {
                                id: id.clone(),
                                method,
                                params: msg.get("params").cloned().unwrap_or(Value::Null),
                            });
                        }
                        (Some(method), _) => {
                            let _ = tx.send(Incoming::Notification {
                                method,
                                params: msg.get("params").cloned().unwrap_or(Value::Null),
                            });
                        }
                        (None, Some(id)) => {
                            if let Some(waiter) =
                                id.as_i64().and_then(|i| pending.lock().unwrap().remove(&i))
                            {
                                let _ = waiter.send(msg);
                            }
                        }
                        (None, None) => {}
                    }
                }
                // Output closed: every waiter learns it through its dropped sender.
                pending.lock().unwrap().clear();
            });
        }
        {
            let tail = tail.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                while let Ok(n) = stderr.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let mut t = tail.lock().unwrap();
                    t.extend(&buf[..n]);
                    let excess = t.len().saturating_sub(STDERR_TAIL);
                    t.drain(..excess);
                }
            });
        }
        Ok((
            Conn {
                stdin: tokio::sync::Mutex::new(stdin),
                pending,
                next: AtomicI64::new(1),
                stderr: tail,
                child: Mutex::new(Some(child)),
                _tree: tree,
            },
            rx,
        ))
    }

    async fn write(&self, msg: &Value) -> Result<(), AcpError> {
        let mut line = msg.to_string();
        line.push('\n');
        let mut w = self.stdin.lock().await;
        w.write_all(line.as_bytes())
            .await
            .map_err(|e| AcpError::Closed(format!(": {e}")))?;
        w.flush()
            .await
            .map_err(|e| AcpError::Closed(format!(": {e}")))
    }

    /// Sends a request and returns the receiver of its response, so the caller can keep serving agent messages
    /// while it runs (`session/prompt` lasts the whole turn).
    pub async fn start_request(
        &self,
        method: &str,
        params: Value,
    ) -> Result<oneshot::Receiver<Value>, AcpError> {
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        Ok(rx)
    }

    /// Reads a response delivered to a receiver from [`Conn::start_request`].
    pub fn result(
        &self,
        response: Result<Value, oneshot::error::RecvError>,
    ) -> Result<Value, AcpError> {
        let v = response.map_err(|_| AcpError::Closed(self.stderr_hint()))?;
        if let Some(e) = v.get("error") {
            return Err(AcpError::Rpc {
                code: e["code"].as_i64().unwrap_or(0),
                message: e["message"].as_str().unwrap_or("").to_owned(),
            });
        }
        v.get("result")
            .cloned()
            .ok_or_else(|| AcpError::Protocol("response without result".into()))
    }

    /// Request with a deadline, for the short calls (`initialize`, `session/new`, ...).
    pub async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, AcpError> {
        let rx = self.start_request(method, params).await?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(r) => self.result(r),
            Err(_) => Err(AcpError::Timeout {
                method: method.into(),
                timeout,
            }),
        }
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), AcpError> {
        self.write(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await
    }

    pub async fn respond(&self, id: Value, result: Value) -> Result<(), AcpError> {
        self.write(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
            .await
    }

    pub async fn respond_error(&self, id: Value, code: i64, message: &str) -> Result<(), AcpError> {
        self.write(
            &json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        )
        .await
    }

    /// Last lines of the CLI's stderr (for error messages).
    pub fn stderr_tail(&self) -> String {
        let t = self.stderr.lock().unwrap();
        let bytes: Vec<u8> = t.iter().copied().collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn stderr_hint(&self) -> String {
        let tail = self.stderr_tail();
        let last: Vec<&str> = tail.lines().rev().take(3).collect();
        if last.is_empty() {
            String::new()
        } else {
            format!(
                " (stderr: {})",
                last.into_iter().rev().collect::<Vec<_>>().join(" | ")
            )
        }
    }

    /// Kills the CLI (and, through the tree guard on drop, its children).
    pub async fn kill(&self) {
        let child = self.child.lock().unwrap().take();
        if let Some(mut c) = child {
            let _ = c.kill().await;
        }
    }
}
