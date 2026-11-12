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

/// A shell command an agent wants to run, or an MCP tool it wants to call (`tool`, ADR-143), waiting for the user's
/// decision in the UI.
#[derive(Debug, Clone, Serialize)]
pub struct CommandApproval {
    pub id: u64,
    pub project: ProjectId,
    pub ticket: Option<TicketKey>,
    pub agent: String,
    pub command: String,
    pub cwd: String,
    pub at: String,
    pub tool: Option<ToolRequest>,
    /// A command to run outside the sandbox (ADR-161): what the sandbox blocked, or why the agent asks.
    pub outside: Option<String>,
}

/// The MCP tool of an approval: its server, its name, the arguments (pretty JSON) and whether the server marks it
/// destructive.
#[derive(Debug, Clone, Serialize)]
pub struct ToolRequest {
    pub server: String,
    pub tool: String,
    pub arguments: String,
    pub destructive: bool,
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
        let req = CommandApproval {
            id: 0,
            project,
            ticket: ticket.cloned(),
            agent: agent.to_owned(),
            command: command.to_owned(),
            cwd: cwd.display().to_string(),
            at: agenium_core::now_iso(),
            tool: None,
            outside: None,
        };
        self.ask(req).await
    }

    async fn approve_tool(
        &self,
        project: ProjectId,
        ticket: Option<&TicketKey>,
        agent: &str,
        ask: &agenium_tools::ToolAsk,
    ) -> Approval {
        let req = CommandApproval {
            id: 0,
            project,
            ticket: ticket.cloned(),
            agent: agent.to_owned(),
            command: format!("MCP {}/{}", ask.server, ask.tool),
            cwd: String::new(),
            at: agenium_core::now_iso(),
            tool: Some(ToolRequest {
                server: ask.server.clone(),
                tool: ask.tool.clone(),
                arguments: serde_json::to_string_pretty(&ask.arguments).unwrap_or_default(),
                destructive: ask.destructive == Some(true),
            }),
            outside: None,
        };
        self.ask(req).await
    }

    async fn approve_escalation(
        &self,
        project: ProjectId,
        ticket: Option<&TicketKey>,
        agent: &str,
        command: &str,
        cwd: &Path,
        why: &str,
    ) -> Approval {
        let req = CommandApproval {
            id: 0,
            project,
            ticket: ticket.cloned(),
            agent: agent.to_owned(),
            command: command.to_owned(),
            cwd: cwd.display().to_string(),
            at: agenium_core::now_iso(),
            tool: None,
            outside: Some(why.to_owned()),
        };
        self.ask(req).await
    }
}

impl UiApprover {
    /// Puts the request in Needs you and waits for the user's answer.
    async fn ask(&self, mut req: CommandApproval) -> Approval {
        let id = self.approvals.next.fetch_add(1, Ordering::SeqCst) + 1;
        req.id = id;
        let (tx, rx) = oneshot::channel();
        self.approvals
            .waiting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, (req.clone(), tx));
        let _withdraw = Withdraw(&self.approvals, id);
        let _ = self.handle.emit("agenium://approval", &req);
        match tokio::time::timeout(std::time::Duration::from_secs(1800), rx).await {
            Ok(Ok(a)) => a,
            _ => Approval::Deny("nobody answered in the app within 30 minutes".into()),
        }
    }
}

/// Takes a request out of Needs you once its run no longer waits for it: answered, unanswered for 30 minutes, or given
/// up because the run was stopped (it stayed there, answering it did nothing).
struct Withdraw<'a>(&'a Approvals, u64);

impl Drop for Withdraw<'_> {
    fn drop(&mut self) {
        self.0
            .waiting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.1);
    }
}

pub struct Opened {
    pub app: Arc<App>,
    pub rt: Arc<Runtime>,
}

/// Models per CLI program (ADR-68 follow-up).
pub type CliModels = std::collections::BTreeMap<String, Vec<agenium_llm::ModelInfo>>;

/// Marks a root set up for a UI test (`AGENIUM_UI_TEST_KEYRING=memory`): only such a root opens with its vault key in
/// memory in a release build (the skill app-ui-testing launches one).
pub const TEST_ROOT_MARK: &str = ".ui-test-root";

/// The UI-test keyring was asked for in this process's environment.
pub fn memory_keyring_asked() -> bool {
    std::env::var("AGENIUM_UI_TEST_KEYRING").as_deref() == Ok("memory")
}

pub struct Desktop {
    /// The Agenium root; set once more when a Linux or macOS first start picks another folder (ADR-154).
    root: RwLock<PathBuf>,
    /// A Linux or macOS first start waits for the user to say which folder to work in.
    asking_root: std::sync::atomic::AtomicBool,
    pub opened: RwLock<Option<Arc<Opened>>>,
    pub approvals: Arc<Approvals>,
    /// The project rounds currently running (no second round of the same project in parallel).
    pub busy: Arc<Mutex<std::collections::HashSet<agenium_core::ProjectId>>>,
    /// The embedding model's preparation: the root opens once it is ready (`crate::model`).
    pub model: Mutex<crate::model::ModelPrep>,
}

impl Desktop {
    pub fn new(root: PathBuf, ask_root: bool) -> Self {
        Self {
            root: RwLock::new(root),
            asking_root: std::sync::atomic::AtomicBool::new(ask_root),
            opened: RwLock::new(None),
            approvals: Arc::default(),
            busy: Arc::default(),
            model: Mutex::default(),
        }
    }

    pub fn root(&self) -> PathBuf {
        self.root.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn asking_root(&self) -> bool {
        self.asking_root.load(Ordering::SeqCst)
    }

    /// The folder the first start was told to work in; nothing is asked after.
    pub fn set_root(&self, root: PathBuf) {
        *self.root.write().unwrap_or_else(|p| p.into_inner()) = root;
        self.asking_root.store(false, Ordering::SeqCst);
    }

    pub fn model_prep(&self) -> crate::model::ModelPrep {
        self.model.lock().unwrap_or_else(|e| e.into_inner()).clone()
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
        // UI tests: a vault key kept in memory, so a throwaway root leaves nothing in the OS keyring. A release build
        // does so only for a root set up that way (`TEST_ROOT_MARK`); any other root uses the OS keyring.
        let keyring: Arc<dyn agenium_secrets::KeyringBackend> = if memory_keyring_asked()
            && (cfg!(debug_assertions) || self.root().join(TEST_ROOT_MARK).exists())
        {
            Arc::new(agenium_secrets::MemoryKeyring::default())
        } else {
            Arc::new(agenium_secrets::OsKeyring::default())
        };
        let (app, report) = App::open(
            &self.root(),
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
        // One model call ended, not the run (an API run makes one per step; a CLI run none): the run's end reaches the
        // live views as its `RunEnded` event.
        E::Done => return None,
    })
}
