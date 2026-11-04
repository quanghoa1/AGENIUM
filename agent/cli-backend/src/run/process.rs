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
    // What the CLI says besides its events (warnings about its model, its login…), for diagnostics.
    tracing::debug!(target: "agenium_cli_backend::stderr", "{}", String::from_utf8_lossy(bytes).trim_end());
    let mut t = tail.lock().unwrap();
    t.extend_from_slice(bytes);
    let excess = t.len().saturating_sub(STDERR_TAIL);
    t.drain(..excess);
}

/// One stdout line of the CLI: a JSON event goes to the run loop (`false` once nobody reads them); anything else is
/// kept with the stderr tail, where a failure is explained from. Read as bytes: a line that is not UTF-8 ended the
/// reader, and the run was told the CLI had exited while it still ran.
fn forward(line: &[u8], out: &mpsc::UnboundedSender<Value>, tail: &Mutex<Vec<u8>>) -> bool {
    match serde_json::from_str::<Value>(String::from_utf8_lossy(line).trim()) {
        Ok(v) => out.send(v).is_ok(),
        Err(_) => {
            if !line.iter().all(u8::is_ascii_whitespace) {
                keep_tail(tail, line);
            }
            true
        }
    }
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

    /// Whether the CLI's own process has exited (a host process; a contained one is not asked). Its output can stay
    /// open after it: a process it started holds the pipe, and the run waited for an end that could not come.
    pub(super) fn exited(&mut self) -> bool {
        match &mut self.handle {
            Handle::Host { child, .. } => matches!(child.try_wait(), Ok(Some(_))),
            Handle::Contained(_) => false,
        }
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
                if self.driver.acp() {
                    let (Ok(w), Ok(r)) = (std_writer(stdin), std_reader(stdout)) else {
                        return Err(failed("stdio pipes could not be handed to the bridge"));
                    };
                    crate::devin::bridge(
                        w,
                        r,
                        in_rx,
                        out_tx,
                        env,
                        self.cwd.clone(),
                        tail.clone(),
                        resume.map(str::to_owned),
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
                if self.driver.served() {
                    drop(stdin);
                    crate::opencode::bridge(
                        stdout,
                        in_rx,
                        out_tx,
                        env,
                        self.cwd.clone(),
                        tail.clone(),
                        resume.map(str::to_owned),
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
                let t = tail.clone();
                tokio::spawn(async move {
                    let mut r = BufReader::new(stdout);
                    let mut line = Vec::new();
                    loop {
                        line.clear();
                        match r.read_until(b'\n', &mut line).await {
                            Ok(0) | Err(_) => break,
                            Ok(_) if !forward(&line, &out_tx, &t) => break,
                            Ok(_) => {}
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
                policy.writable_roots.extend(self.driver.writable_roots());
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
                if self.driver.acp() {
                    crate::devin::bridge(
                        stdin,
                        stdout,
                        in_rx,
                        out_tx,
                        env,
                        self.cwd.clone(),
                        tail.clone(),
                        resume.map(str::to_owned),
                    );
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
                    return Ok(Proc {
                        input: Some(in_tx),
                        lines,
                        stderr: tail,
                        handle: Handle::Contained(Arc::new(Mutex::new(child))),
                    });
                }
                let mut in_rx = in_rx;
                std::thread::spawn(move || {
                    while let Some(l) = in_rx.blocking_recv() {
                        if stdin.write_all(l.as_bytes()).is_err() || stdin.flush().is_err() {
                            break;
                        }
                    }
                });
                let t = tail.clone();
                std::thread::spawn(move || {
                    let mut r = std::io::BufReader::new(stdout);
                    let mut line = Vec::new();
                    loop {
                        line.clear();
                        match std::io::BufRead::read_until(&mut r, b'\n', &mut line) {
                            Ok(0) | Err(_) => break,
                            Ok(_) if !forward(&line, &out_tx, &t) => break,
                            Ok(_) => {}
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

/// A host child's stdin as a plain writer, for a bridge that runs on threads (the ACP bridge).
fn std_writer(s: tokio::process::ChildStdin) -> std::io::Result<Box<dyn std::io::Write + Send>> {
    #[cfg(windows)]
    let f = std::fs::File::from(s.into_owned_handle()?);
    #[cfg(unix)]
    let f = std::fs::File::from(s.into_owned_fd()?);
    Ok(Box::new(f))
}

/// A host child's stdout as a plain reader (see [`std_writer`]).
fn std_reader(s: tokio::process::ChildStdout) -> std::io::Result<Box<dyn std::io::Read + Send>> {
    #[cfg(windows)]
    let f = std::fs::File::from(s.into_owned_handle()?);
    #[cfg(unix)]
    let f = std::fs::File::from(s.into_owned_fd()?);
    Ok(Box::new(f))
}
