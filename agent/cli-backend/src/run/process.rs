//! The CLI's process: started in the sandbox, fed on stdin, its stderr kept, stopped.

use super::*;

pub(super) enum Handle {
    Host {
        child: Box<tokio::process::Child>,
        _tree: TreeGuard,
    },
    Contained(Arc<Mutex<Contained>>),
}

/// One running CLI process: stdin through a writer, stdout as JSON values, a tail of stderr.
pub(super) struct Proc {
    pub(super) input: Option<mpsc::UnboundedSender<String>>,
    pub(super) lines: mpsc::UnboundedReceiver<Value>,
    pub(super) stderr: Arc<Mutex<Vec<u8>>>,
    pub(super) handle: Handle,
}

pub(crate) fn keep_tail(tail: &Mutex<Vec<u8>>, bytes: &[u8]) {
    let mut t = tail.lock().unwrap();
    t.extend_from_slice(bytes);
    let excess = t.len().saturating_sub(STDERR_TAIL);
    t.drain(..excess);
}

impl Proc {
    pub(super) fn send(&self, text: &str) -> Result<(), String> {
        let mut l = text.to_owned();
        l.push('\n');
        self.input
            .as_ref()
            .ok_or_else(|| "the CLI's input is closed".to_owned())?
            .send(l)
            .map_err(|_| "the CLI's input is closed".to_owned())
    }

    pub(super) fn close_input(&mut self) {
        self.input = None;
    }

    pub(super) fn stderr_tail(&self) -> String {
        let b = self.stderr.lock().unwrap();
        let s = String::from_utf8_lossy(&b);
        let mut last: Vec<&str> = s
            .lines()
            .rev()
            .filter(|l| !l.trim().is_empty())
            .take(3)
            .collect();
        last.reverse();
        last.join(" | ")
    }

    pub(super) async fn stop(&mut self) {
        self.input = None;
        match &mut self.handle {
            Handle::Host { child, .. } => {
                let _ = child.kill().await;
            }
            Handle::Contained(c) => {
                let c = c.clone();
                let _ = tokio::task::spawn_blocking(move || c.lock().unwrap().kill()).await;
            }
        }
    }

    /// Waits for a per-turn process to exit by itself, then makes sure it is gone.
    pub(super) async fn finish(&mut self) {
        self.input = None;
        let _ = tokio::time::timeout(EXIT_GRACE, async {
            while self.lines.recv().await.is_some() {}
        })
        .await;
        self.stop().await;
    }
}

impl CliRun<'_> {
    pub(super) fn spawn(
        &self,
        env: &BTreeMap<String, String>,
        home: &std::path::Path,
        resume: Option<&str>,
    ) -> Result<Proc, Outcome> {
        let args = self.driver.args(
            &self.model,
            self.effort.as_ref(),
            home,
            resume,
            self.subagents,
        );
        let name = self.program.display().to_string();
        let (in_tx, in_rx) = mpsc::unbounded_channel::<String>();
        let (out_tx, lines) = mpsc::unbounded_channel::<Value>();
        let tail: Arc<Mutex<Vec<u8>>> = Arc::default();
        match (self.driver.control(), self.uncontained) {
            (Control::Hook, _) | (Control::Sandbox, true) => {
                let mut child = agenium_sandbox::service_command(&name, &args, env, &self.cwd)
                    .spawn()
                    .map_err(|e| failed(format!("cannot start {name}: {e}")))?;
                let tree = TreeGuard::attach(&child);
                let (Some(mut stdin), Some(stdout), Some(mut stderr)) =
                    (child.stdin.take(), child.stdout.take(), child.stderr.take())
                else {
                    return Err(failed("stdio pipes missing"));
                };
                if self.driver.served() {
                    drop(stdin);
                    crate::opencode::bridge(
                        stdout,
                        in_rx,
                        out_tx,
                        env,
                        self.cwd.clone(),
                        tail.clone(),
                    );
                    let t = tail.clone();
                    tokio::spawn(async move {
                        let mut buf = [0u8; 4096];
                        while let Ok(n) = stderr.read(&mut buf).await {
                            if n == 0 {
                                break;
                            }
                            keep_tail(&t, &buf[..n]);
                        }
                    });
                    return Ok(Proc {
                        input: Some(in_tx),
                        lines,
                        stderr: tail,
                        handle: Handle::Host {
                            child: Box::new(child),
                            _tree: tree,
                        },
                    });
                }
                let mut in_rx = in_rx;
                tokio::spawn(async move {
                    while let Some(l) = in_rx.recv().await {
                        if stdin.write_all(l.as_bytes()).await.is_err()
                            || stdin.flush().await.is_err()
                        {
                            break;
                        }
                    }
                });
                tokio::spawn(async move {
                    let mut r = BufReader::new(stdout).lines();
                    while let Ok(Some(line)) = r.next_line().await {
                        if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
                            if out_tx.send(v).is_err() {
                                break;
                            }
                        }
                    }
                });
                let t = tail.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    while let Ok(n) = stderr.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        keep_tail(&t, &buf[..n]);
                    }
                });
                Ok(Proc {
                    input: Some(in_tx),
                    lines,
                    stderr: tail,
                    handle: Handle::Host {
                        child: Box::new(child),
                        _tree: tree,
                    },
                })
            }
            (Control::Sandbox, false) => {
                let mut policy = self.ctx.policy.clone();
                policy.network = true;
                policy.writable_roots.push(home.to_path_buf());
                policy
                    .read_roots
                    .extend(self.driver.read_roots(&self.program));
                let mut argv = vec![name.clone()];
                argv.extend(args);
                let (child, io) = spawn_contained(&argv, &self.cwd, env, &policy)
                    .map_err(|e| failed(format!("cannot start {name} in the sandbox: {e}")))?;
                let (Some(mut stdin), Some(stdout), Some(mut stderr)) =
                    (io.stdin, io.stdout, io.stderr)
                else {
                    return Err(failed("stdio pipes missing"));
                };
                let mut in_rx = in_rx;
                std::thread::spawn(move || {
                    while let Some(l) = in_rx.blocking_recv() {
                        if stdin.write_all(l.as_bytes()).is_err() || stdin.flush().is_err() {
                            break;
                        }
                    }
                });
                std::thread::spawn(move || {
                    for line in std::io::BufReader::new(stdout).lines() {
                        let Ok(line) = line else { break };
                        if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
                            if out_tx.send(v).is_err() {
                                break;
                            }
                        }
                    }
                });
                let t = tail.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    while let Ok(n) = stderr.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        keep_tail(&t, &buf[..n]);
                    }
                });
                Ok(Proc {
                    input: Some(in_tx),
                    lines,
                    stderr: tail,
                    handle: Handle::Contained(Arc::new(Mutex::new(child))),
                })
            }
        }
    }
}
