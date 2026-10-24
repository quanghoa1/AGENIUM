//! Agent work tools (decision 16). Descriptions and schemas are Agenium's own (English, byte-stable for the
//! prompt cache). Behaviour rules: paths confined to the ticket worktree, read-before-edit with freshness check,
//! unique match for edits, atomic multi-file patches, outputs cut to 30 000 characters / 256 lines with the full
//! text saved as an artifact, shell commands through the command gate (approval or sandbox) with timeouts.

pub mod patch;
pub mod paths;
pub mod text;
mod work;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use work::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ToolError {
    pub code: String,
    pub message: String,
}

impl ToolError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolOutput {
    /// Text returned to the model (already truncated).
    pub text: String,
    pub truncated: bool,
    /// Worktree-relative path of the full output when it was cut.
    pub full: Option<String>,
    /// Content came from outside the trust boundary (fetch, MCP); the harness labels it untrusted (decision 34).
    pub untrusted: bool,
}

impl ToolOutput {
    pub fn text(t: impl Into<String>) -> Self {
        Self {
            text: t.into(),
            truncated: false,
            full: None,
            untrusted: false,
        }
    }
}

/// Decision about running an agent command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    Allow,
    Deny(String),
}

/// Who decides whether a shell command may run: the user (approval mode, before the sandbox exists on this OS)
/// or the sandbox policy (YOLO only with a working backend). Implemented by the runtime.
#[async_trait]
pub trait CommandGate: Send + Sync {
    async fn check(&self, command: &str, cwd: &Path) -> GateDecision;
}

/// A gate that refuses everything (headless runs without an approver).
pub struct DenyAll(pub String);

#[async_trait]
impl CommandGate for DenyAll {
    async fn check(&self, _: &str, _: &Path) -> GateDecision {
        GateDecision::Deny(self.0.clone())
    }
}

/// A gate that allows everything; tests only.
pub struct AllowAll;

#[async_trait]
impl CommandGate for AllowAll {
    async fn check(&self, _: &str, _: &Path) -> GateDecision {
        GateDecision::Allow
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_chars: usize,
    pub max_lines: usize,
    pub shell_timeout: Duration,
    pub shell_timeout_max: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_chars: 30_000,
            max_lines: 256,
            shell_timeout: Duration::from_secs(120),
            shell_timeout_max: Duration::from_secs(600),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub text: String,
    pub status: TodoStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Done,
}

/// Everything a tool call may touch. One per run.
pub struct ToolCtx {
    /// Ticket worktree: the only writable root.
    pub root: PathBuf,
    /// Extra read-only roots (skill folders).
    pub read_roots: Vec<PathBuf>,
    /// Worktree-relative directory for full outputs (`artifacts/tool-output/<run>`).
    pub artifacts_rel: String,
    pub limits: Limits,
    pub executor: agenium_sandbox::Executor,
    pub policy: agenium_sandbox::SandboxPolicy,
    pub gate: Arc<dyn CommandGate>,
    /// Extra environment for commands (e.g. `AGENIUM_RUN_ID`).
    pub env: std::collections::BTreeMap<String, String>,
    /// Domains `fetch` may reach (project allowlist + grants); `*.x` wildcards.
    pub fetch_allow: Vec<String>,
    /// Extra lines for the capability manifest (connected MCP servers and their problems).
    pub capabilities: Vec<String>,
    /// Write scope inside the worktree (open by default).
    pub write_scope: paths::WriteScope,
    pub http: reqwest::Client,
    ledger: Mutex<HashMap<PathBuf, [u8; 32]>>,
    todo: Mutex<Vec<TodoItem>>,
    seq: AtomicU32,
}

impl ToolCtx {
    pub fn new(root: &Path, run: &str, gate: Arc<dyn CommandGate>) -> Self {
        let policy = agenium_sandbox::SandboxPolicy::workspace(root);
        let mut env = std::collections::BTreeMap::new();
        env.insert("AGENIUM_RUN_ID".to_owned(), run.to_owned());
        Self {
            root: root.to_path_buf(),
            read_roots: vec![],
            artifacts_rel: format!("artifacts/tool-output/{run}"),
            limits: Limits::default(),
            executor: agenium_sandbox::Executor::new(agenium_sandbox::Shell::detect()),
            policy,
            gate,
            env,
            fetch_allow: vec![],
            capabilities: vec![],
            write_scope: paths::WriteScope::default(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            ledger: Mutex::new(HashMap::new()),
            todo: Mutex::new(vec![]),
            seq: AtomicU32::new(0),
        }
    }

    pub fn resolve(&self, raw: &str, write: bool) -> Result<PathBuf, ToolError> {
        let p = paths::resolve(&self.root, &self.read_roots, raw, write)?;
        if write && !self.write_scope.is_open() {
            let rel = paths::display(&self.root, &p);
            if !self.write_scope.allows(&rel) {
                return Err(ToolError::new(
                    "out_of_scope",
                    format!(
                        "{rel} is outside the write scope of your ticket and role {}; if the work needs it,                          request_permission (kind=path, paths=[\"{rel}\"]) or ask_question",
                        self.write_scope.describe()
                    ),
                ));
            }
        }
        Ok(p)
    }

    pub(crate) fn record_read(&self, p: &Path, bytes: &[u8]) {
        self.ledger
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(p.to_path_buf(), text::digest(bytes));
    }

    /// Read-before-edit: the file must have been read in this run and not changed since.
    pub(crate) fn check_fresh(&self, p: &Path, current: &[u8], rel: &str) -> Result<(), ToolError> {
        match self.ledger.lock().unwrap_or_else(|e| e.into_inner()).get(p) {
            None => Err(ToolError::new(
                "read_first",
                format!("read {rel} with read_file before modifying it"),
            )),
            Some(h) if *h != text::digest(current) => Err(ToolError::new(
                "stale",
                format!("{rel} changed since you read it; read it again before modifying it"),
            )),
            Some(_) => Ok(()),
        }
    }

    /// Cuts `full` to the limits; when cut, saves the whole text under the artifacts directory.
    pub fn finish_output(&self, full: String, untrusted: bool) -> ToolOutput {
        let (text, truncated) = text::truncate(&full, self.limits.max_chars, self.limits.max_lines);
        let mut saved = None;
        if truncated {
            let n = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
            let rel = format!("{}/{n:04}.log", self.artifacts_rel);
            let p = self.root.join(&rel);
            if p.parent().map(std::fs::create_dir_all).transpose().is_ok()
                && std::fs::write(&p, &full).is_ok()
            {
                saved = Some(rel);
            }
        }
        let text = match &saved {
            Some(rel) => format!("{text}\n[full output: {rel}]"),
            None => text,
        };
        ToolOutput {
            text,
            truncated,
            full: saved,
            untrusted,
        }
    }

    pub fn todo(&self) -> Vec<TodoItem> {
        self.todo.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn set_todo(&self, items: Vec<TodoItem>) {
        *self.todo.lock().unwrap_or_else(|e| e.into_inner()) = items;
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;
    /// Read-only tools may run in parallel.
    fn read_only(&self) -> bool {
        false
    }
    async fn call(&self, ctx: &ToolCtx, args: serde_json::Value) -> Result<ToolOutput, ToolError>;
}

pub(crate) fn args<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> Result<T, ToolError> {
    let v = if v.is_null() {
        serde_json::json!({})
    } else {
        v
    };
    serde_json::from_value(v).map_err(|e| ToolError::new("invalid_arguments", e.to_string()))
}

/// The work tools, sorted by name (stable tool list for the prompt cache).
pub fn work_tools() -> Vec<Arc<dyn Tool>> {
    let mut v: Vec<Arc<dyn Tool>> = vec![
        Arc::new(ReadFile),
        Arc::new(ListDir),
        Arc::new(GlobTool),
        Arc::new(GrepTool),
        Arc::new(WriteFile),
        Arc::new(EditFile),
        Arc::new(MultiEdit),
        Arc::new(ApplyPatch),
        Arc::new(ShellTool),
        Arc::new(FetchTool),
        Arc::new(TodoTool),
    ];
    v.sort_by(|a, b| a.name().cmp(b.name()));
    v
}
