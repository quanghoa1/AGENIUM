//! The desktop process state: the Agenium root (opened once set up), the runtime, the command approvals that wait
//! for the user, and the projects whose team round is running.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use agenium_core::{ProjectId, RunId, TicketKey};
use agenium_orchestrator::app::{App, AppOptions};
use agenium_orchestrator::check::SandboxCheckFactory;
use agenium_orchestrator::runtime::{Approval, Approver, ConfiguredProviders, Runtime};
use async_trait::async_trait;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

pub type R<T> = Result<T, String>;

pub fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A shell command an agent wants to run, waiting for the user's decision in the UI.
#[derive(Debug, Clone, Serialize)]
pub struct CommandApproval {
    pub id: u64,
    pub project: ProjectId,
    pub ticket: Option<TicketKey>,
    pub agent: String,
    pub command: String,
    pub cwd: String,
    pub at: String,
}

#[derive(Default)]
pub struct Approvals {
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, (CommandApproval, oneshot::Sender<Approval>)>>,
}

impl Approvals {
    /// The requests of one project, or of every project.
    pub fn pending(&self, project: Option<ProjectId>) -> Vec<CommandApproval> {
        let mut v: Vec<_> = self
            .waiting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|(a, _)| project.is_none_or(|p| a.project == p))
            .map(|(a, _)| a.clone())
            .collect();
        v.sort_by_key(|a| a.id);
        v
    }

    pub fn decide(&self, id: u64, a: Approval) -> bool {
        match self
            .waiting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id)
        {
            Some((_, tx)) => tx.send(a).is_ok(),
            None => false,
        }
    }
}

/// Asks the user in the UI (event `agenium://approval`); unanswered requests are denied after 30 minutes.
pub struct UiApprover {
    pub approvals: Arc<Approvals>,
    pub handle: AppHandle,
}

#[async_trait]
impl Approver for UiApprover {
    async fn approve_command(
        &self,
        project: ProjectId,
        ticket: Option<&TicketKey>,
        agent: &str,
        command: &str,
        cwd: &Path,
    ) -> Approval {
        let id = self.approvals.next.fetch_add(1, Ordering::SeqCst) + 1;
        let req = CommandApproval {
            id,
            project,
            ticket: ticket.cloned(),
            agent: agent.to_owned(),
            command: command.to_owned(),
            cwd: cwd.display().to_string(),
            at: agenium_core::now_iso(),
        };
        let (tx, rx) = oneshot::channel();
        self.approvals
            .waiting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, (req.clone(), tx));
        let _ = self.handle.emit("agenium://approval", &req);
        match tokio::time::timeout(std::time::Duration::from_secs(1800), rx).await {
            Ok(Ok(a)) => a,
            _ => {
                self.approvals
                    .waiting
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&id);
                Approval::Deny("nobody answered in the app within 30 minutes".into())
            }
        }
    }
}

pub struct Opened {
    pub app: Arc<App>,
    pub rt: Arc<Runtime>,
}

/// Models per CLI program (ADR-68 follow-up).
pub type CliModels = std::collections::BTreeMap<String, Vec<String>>;

pub struct Desktop {
    pub root: PathBuf,
    pub opened: RwLock<Option<Arc<Opened>>>,
    pub approvals: Arc<Approvals>,
    /// The project rounds currently running (no second round of the same project in parallel).
    pub busy: Arc<Mutex<std::collections::HashSet<agenium_core::ProjectId>>>,
    /// Agent CLIs found on this machine and when (detection runs each CLI: cached, refreshed on demand).
    pub clis: Mutex<Option<(std::time::Instant, Vec<agenium_cli_backend::CliInfo>)>>,
    /// Models each CLI can run, listed at most every 5 minutes.
    pub cli_models: Mutex<Option<(std::time::Instant, CliModels)>>,
}

impl Desktop {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            opened: RwLock::new(None),
            approvals: Arc::default(),
            busy: Arc::default(),
            clis: Mutex::new(None),
            cli_models: Mutex::new(None),
        }
    }

    pub fn opened(&self) -> R<Arc<Opened>> {
        self.opened
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| "Agenium is not set up yet".to_owned())
    }

    /// Opens the root (reconcile included) and builds the runtime; live run output goes to `agenium://run`.
    pub fn open(&self, handle: &AppHandle) -> R<Arc<Opened>> {
        // UI tests (debug builds only): a vault key kept in memory, so a throwaway root leaves nothing in the OS
        // keyring. A release build always uses the OS keyring.
        let keyring: Arc<dyn agenium_secrets::KeyringBackend> = if cfg!(debug_assertions)
            && std::env::var("AGENIUM_UI_TEST_KEYRING").as_deref() == Ok("memory")
        {
            Arc::new(agenium_secrets::MemoryKeyring::default())
        } else {
            Arc::new(agenium_secrets::OsKeyring::default())
        };
        let (app, report) = App::open(
            &self.root,
            AppOptions {
                keyring,
                // Pre-merge checks need an approval UI of their own: not run from the desktop app yet.
                checks: Arc::new(SandboxCheckFactory),
            },
        )
        .map_err(err)?;
        for a in &report.actions {
            tracing::info!("[reconcile] {a}");
        }
        let app = Arc::new(app);
        let mut rt = Runtime::new(
            app.clone(),
            Arc::new(ConfiguredProviders::new(app.clone())),
            Arc::new(UiApprover {
                approvals: self.approvals.clone(),
                handle: handle.clone(),
            }),
        );
        let h = handle.clone();
        // Each event names the run's agent (looked up once per run) so a chat shows only its own agent's turn.
        let store = app.store().clone();
        let agents = std::sync::Mutex::new(std::collections::HashMap::<RunId, String>::new());
        rt.live = Some(Arc::new(move |run: &RunId, e: &agenium_llm::ChatEvent| {
            let Some(v) = live_json(e) else { return };
            let mut known = agents.lock().unwrap_or_else(|p| p.into_inner());
            if !known.contains_key(run) {
                if let Ok(r) = store.read(|c| agenium_store::core::runs::get(c, *run)) {
                    if known.len() > 512 {
                        known.clear();
                    }
                    known.insert(*run, r.agent.to_string());
                }
            }
            let agent = known.get(run).cloned();
            drop(known);
            let _ = h.emit(
                "agenium://run",
                serde_json::json!({ "run": run, "agent": agent, "event": v }),
            );
        }));
        let o = Arc::new(Opened {
            app,
            rt: Arc::new(rt),
        });
        *self.opened.write().unwrap_or_else(|p| p.into_inner()) = Some(o.clone());
        Ok(o)
    }
}

/// Live run output for the agent view (deltas are small; usage arrives per step).
fn live_json(e: &agenium_llm::ChatEvent) -> Option<serde_json::Value> {
    use agenium_llm::ChatEvent as E;
    Some(match e {
        E::TextDelta(t) => serde_json::json!({ "type": "text", "text": t }),
        E::ThinkingDelta(t) => serde_json::json!({ "type": "thinking", "text": t }),
        E::ToolCallStarted { id, name } => {
            serde_json::json!({ "type": "tool_start", "id": id, "name": name })
        }
        E::ToolCall(c) => {
            serde_json::json!({ "type": "tool", "id": c.id, "name": c.name, "args": c.args })
        }
        E::Usage(u) => serde_json::json!({ "type": "usage", "input": u.input, "output": u.output }),
        E::Restart => serde_json::json!({ "type": "restart" }),
        E::Done => serde_json::json!({ "type": "done" }),
    })
}
