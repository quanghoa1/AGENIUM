//! The stdio transport: a child process speaking JSON-RPC lines.

use super::*;

pub(super) struct Stdio {
    stdin: Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>,
    pending: Pending,
    next: AtomicI64,
    alive: Arc<AtomicBool>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    _child: Mutex<tokio::process::Child>,
    _guard: agenium_sandbox::TreeGuard,
}

pub(super) async fn write_line(
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
    pub(super) fn spawn(
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

    pub(super) fn stderr_tail(&self) -> String {
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
