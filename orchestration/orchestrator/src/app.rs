//! The application service: owns the root, configuration, `core.db`, the realtime bus and the secret store, and
//! exposes every state-changing operation. UI/CLI/agent runtimes call only this.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use agenium_bus::{Bus, EventSource};
use agenium_config::{Config, RootLayout, SecretsBackend};
use agenium_core::*;
use agenium_secrets::{KeyringBackend, Redactor, SecretStore};
use agenium_store::core::{agents, events, projects, tickets};
use agenium_store::{AgentStore, CoreStore, StoreError};
use agenium_vcs::{CheckOutcome, CheckRunner, NoCheck, Repo};
use serde::{Deserialize, Serialize};

use crate::manifest::ProjectManifest;
use crate::{OrchError, Result};

/// Builds the pre-merge check of a project. The runtime decides how commands run (sandbox or approval); tests
/// and projects without a check use [`NoCheckFactory`]. `network`: `[sandbox] network`, as the work's own commands
/// had it (a test suite that starts its own server needs the loopback, which the sandbox cannot tell from the host's).
pub trait CheckFactory: Send + Sync {
    fn for_project(
        &self,
        project: &projects::Project,
        manifest: &ProjectManifest,
        network: bool,
    ) -> Arc<dyn CheckRunner>;
}

pub struct NoCheckFactory;
impl CheckFactory for NoCheckFactory {
    fn for_project(
        &self,
        _: &projects::Project,
        _: &ProjectManifest,
        _: bool,
    ) -> Arc<dyn CheckRunner> {
        Arc::new(NoCheck)
    }
}

/// A check whose result is fixed (tests).
pub struct FixedCheck(pub bool, pub String);
impl CheckRunner for FixedCheck {
    fn run(&self, _: &Path) -> CheckOutcome {
        CheckOutcome {
            ok: self.0,
            log: self.1.clone(),
        }
    }
}

pub struct AppOptions {
    pub keyring: Arc<dyn KeyringBackend>,
    pub checks: Arc<dyn CheckFactory>,
}

/// Test hook: makes a journaled VCS operation fail at a named point (`before:<op>` = after the journal entry,
/// before git; `after:<op>` = after git, before completion is recorded), like a crash at that instant.
pub type FaultPlan = Arc<Mutex<Vec<String>>>;

pub struct App {
    pub(crate) layout: RootLayout,
    pub(crate) config: RwLock<Config>,
    pub(crate) store: Arc<CoreStore>,
    pub(crate) bus: Bus,
    pub(crate) secrets: Arc<dyn SecretStore>,
    pub(crate) redactor: Redactor,
    pub(crate) checks: Arc<dyn CheckFactory>,
    pub(crate) merge_locks: Mutex<HashMap<ProjectId, Arc<Mutex<()>>>>,
    pub(crate) faults: FaultPlan,
    /// Journaled operations whose replays keep failing (`replay_pending_ops` backs off on them).
    pub(crate) op_retries: Mutex<HashMap<i64, crate::effects::OpRetry>>,
    agent_stores: Mutex<HashMap<AgentId, Arc<AgentStore>>>,
    /// Each project's index of its closed tickets' traces (ADR-158 decision 31).
    trace_stores: Mutex<HashMap<ProjectId, Arc<AgentStore>>>,
    /// The judge's client, kept while its endpoint, model and key stay the same (connections reused, ADR-75).
    pub(crate) judge_client: Mutex<crate::judge::KeptJudge>,
    /// Whose turn it is among the judge keys in use (they take turns, one request each).
    pub(crate) judge_turn: std::sync::atomic::AtomicUsize,
    /// Until when the judge is not asked after a failure that was not a key's (`judge::client`).
    pub(crate) judge_paused: Mutex<Option<std::time::Instant>>,
    /// Serializes writes of `<root>/vendor-usage.json`.
    pub(crate) vendor_usage_lock: Mutex<()>,
    /// The last price list read and the file times it was read at ([`App::prices`]).
    pub(crate) prices_cache:
        Mutex<Option<(crate::prices::PricesStamp, agenium_llm::pricing::Prices)>>,
}

struct StoreSource(Arc<CoreStore>);
impl EventSource for StoreSource {
    fn since(&self, after: u64, limit: usize) -> std::result::Result<Vec<EventRecord>, String> {
        self.0
            .read(|c| events::since(c, after, limit))
            .map_err(|e| e.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProject {
    pub name: String,
    /// Folder of the project; default `<root>/projects/<folder>`. May be an existing repository (CLI).
    pub path: Option<PathBuf>,
    pub key_prefix: Option<String>,
    pub template: Option<String>,
    /// Folder name under `<root>/projects` (default: the project name). The app decides where projects live.
    #[serde(default)]
    pub folder: Option<String>,
    /// A git repository to clone into the project folder (URL or `git@host:owner/repo`).
    #[serde(default)]
    pub repo: Option<String>,
}

/// Whether a project's "folder or repo" text names a git repository rather than a folder.
pub fn is_repo_url(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("https://")
        || s.starts_with("http://")
        || s.starts_with("git@")
        || s.starts_with("ssh://")
        || s.ends_with(".git")
}

/// Folder name for a repository URL: its last segment without `.git`.
pub fn repo_folder(url: &str) -> String {
    let last = url
        .trim()
        .trim_end_matches('/')
        .rsplit(['/', '\\', ':'])
        .next()
        .unwrap_or("");
    last.trim_end_matches(".git").to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateAgent {
    pub name: String,
    pub role: String,
    pub role_kind: RoleKind,
    pub tags: Vec<String>,
    pub model: ModelBinding,
}

/// Standard project layout (decision 11).
pub const PROJECT_DIRS: &[&str] = &["deliverables", "artifacts", "notes"];

impl App {
    /// Opens the application on `root`: creates the layout and a default `config.toml` when missing, opens and
    /// migrates `core.db`, wires the bus, opens the secret store, then reconciles DB <-> git.
    pub fn open(root: &Path, opts: AppOptions) -> Result<(App, crate::effects::ReconcileReport)> {
        let layout = RootLayout::new(root);
        layout.ensure()?;
        let config = if layout.config().is_file() {
            Config::load(&layout.config())?
        } else {
            let c = Config::default();
            c.save(&layout.config())?;
            c
        };
        let store = Arc::new(CoreStore::open(&layout.core_db(), &layout.backups())?);
        let bus = Bus::new(4096);
        store.set_sink(Some(Arc::new(bus.clone())));
        let backend = match config.secrets.backend {
            SecretsBackend::Vault => agenium_secrets::Backend::Vault,
            SecretsBackend::Keyring => agenium_secrets::Backend::Keyring,
        };
        let root_id = root_id(&layout)?;
        let secrets = agenium_secrets::open_store(backend, opts.keyring, &layout.vault(), &root_id);
        let app = App {
            layout,
            config: RwLock::new(config),
            store,
            bus,
            secrets,
            redactor: Redactor::new(),
            checks: opts.checks,
            merge_locks: Mutex::new(HashMap::new()),
            faults: Arc::new(Mutex::new(Vec::new())),
            op_retries: Mutex::new(HashMap::new()),
            agent_stores: Mutex::new(HashMap::new()),
            trace_stores: Mutex::new(HashMap::new()),
            judge_client: Mutex::default(),
            judge_turn: Default::default(),
            judge_paused: Mutex::default(),
            vendor_usage_lock: Mutex::new(()),
            prices_cache: Mutex::default(),
        };
        // The tools prefer the markitdown Agenium installed for this root (ADR-152).
        agenium_documents::markitdown::own::use_dir(app.layout.tools().join("markitdown"));
        // The opencode CLI's runs read their model list from the root's catalogue (refreshed with the prices).
        agenium_cli_backend::opencode::use_models_catalogue(app.layout.models_dev());
        let report = app.reconcile()?;
        app.purge_deleted_agents()?;
        Ok((app, report))
    }

    /// The embedding model for agent memories (local model when installed in `<root>/models`).
    pub fn embedder(&self) -> Arc<dyn agenium_memory::Embedder> {
        agenium_memory::default_embedder(&self.layout.models())
    }

    /// Skill folders of agents and roles (`<root>/agents/<id>/skills`, `<root>/roles/<role>/skills`).
    pub fn skill_registry(&self) -> agenium_skills::Registry {
        agenium_skills::Registry::new(self.layout.agents(), self.layout.roles())
    }

    pub fn layout(&self) -> &RootLayout {
        &self.layout
    }

    pub fn store(&self) -> &Arc<CoreStore> {
        &self.store
    }

    pub fn bus(&self) -> &Bus {
        &self.bus
    }

    pub fn config(&self) -> Config {
        self.config
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn update_config(&self, f: impl FnOnce(&mut Config)) -> Result<Config> {
        let mut c = self.config.write().unwrap_or_else(|p| p.into_inner());
        let mut next = c.clone();
        f(&mut next);
        next.check().map_err(OrchError::Invalid)?;
        next.save(&self.layout.config())?;
        *c = next.clone();
        Ok(next)
    }

    pub fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }

    pub fn redactor(&self) -> &Redactor {
        &self.redactor
    }

    pub fn faults(&self) -> &FaultPlan {
        &self.faults
    }

    /// A subscriber over the bus that first replays the log after `after`.
    pub fn subscribe(&self, after: u64) -> agenium_bus::Subscriber {
        self.bus
            .subscribe(Arc::new(StoreSource(self.store.clone())), after)
    }

    /// Reads a secret and registers it with the redactor so it never appears in logs.
    pub fn secret(&self, name: &str) -> Result<Option<agenium_secrets::Secret>> {
        let s = self.secrets.get(name)?;
        if let Some(v) = &s {
            if let Ok(t) = v.expose_str() {
                self.redactor.register(name, t);
            }
        }
        Ok(s)
    }

    // ------------------------------------------------------------ projects

    pub fn projects_dir(&self) -> PathBuf {
        let d = PathBuf::from(&self.config().paths.projects_dir);
        if d.is_absolute() {
            d
        } else {
            self.layout.root.join(d)
        }
    }

    pub fn project(&self, id: ProjectId) -> Result<projects::Project> {
        Ok(self.store.read(|c| projects::get(c, id))?)
    }

    pub fn projects(&self) -> Result<Vec<projects::Project>> {
        Ok(self.store.read(projects::list)?)
    }

    pub fn manifest(&self, p: &projects::Project) -> Result<ProjectManifest> {
        ProjectManifest::load(Path::new(&p.path))?
            .ok_or_else(|| OrchError::NotFound(format!("{} has no agenium.toml", p.path)))
    }

    pub fn repo(&self, p: &projects::Project) -> Repo {
        Repo::open(Path::new(&p.path), &p.main_branch)
    }

    pub fn set_project_status(&self, id: ProjectId, status: projects::ProjectStatus) -> Result<()> {
        self.set_project_status_as(id, status, "user")
    }

    /// The status change as `actor` says it: the system's own (a done project completing, new work making it active
    /// again) is not the user's in the audit trail.
    pub fn set_project_status_as(
        &self,
        id: ProjectId,
        status: projects::ProjectStatus,
        actor: &str,
    ) -> Result<()> {
        self.store
            .commit(None, actor, |w| -> std::result::Result<_, StoreError> {
                projects::set_status(w.tx, id, status)?;
                w.emit(DomainEvent::ProjectStatusChanged {
                    project: id,
                    status: status.as_str().into(),
                })?;
                Ok(())
            })?;
        Ok(())
    }

    /// Project-level settings (e.g. `review_before_merge`).
    pub fn set_project_setting(
        &self,
        id: ProjectId,
        key: &str,
        value: serde_json::Value,
    ) -> Result<()> {
        self.store
            .commit(None, "user", |w| -> std::result::Result<_, StoreError> {
                let mut s = projects::get(w.tx, id)?.settings;
                if !s.is_object() {
                    s = serde_json::json!({});
                }
                s[key] = value;
                projects::set_settings(w.tx, id, &s)
            })?;
        Ok(())
    }

    // ------------------------------------------------------------ registry

    /// Creates a Solo team (one agent acting as Lead + Worker) bound to `project` (decision 6/25).
    pub fn create_solo(
        &self,
        project: ProjectId,
        a: &CreateAgent,
    ) -> Result<(agents::Team, agents::Agent)> {
        self.check_names_free(&[&a.name])?;
        let team = agents::Team {
            id: TeamId::new(),
            name: format!("{} (solo)", a.name),
            template: self.manifest(&self.project(project)?)?.project.template,
            project: Some(project),
            lead: None,
            solo: true,
            created_at: now_iso(),
        };
        let agent = self.new_agent_row(team.id, a, RoleKind::Lead);
        self.store.commit(None, "user", |w| -> Result<()> {
            if agents::teams_of_project(w.tx, project)?
                .iter()
                .any(|t| t.project == Some(project))
            {
                return Err(OrchError::Conflict(
                    "the project already has a team (one team per project, decision 25)".into(),
                ));
            }
            agents::insert_team(w.tx, &team)?;
            agents::insert_agent(w.tx, &agent)?;
            agents::set_team_lead(w.tx, team.id, Some(agent.id))?;
            w.emit(DomainEvent::TeamCreated {
                team: team.id,
                name: team.name.clone(),
                solo: true,
            })?;
            w.emit(DomainEvent::AgentCreated {
                agent: agent.id,
                name: agent.name.clone(),
                team: Some(team.id),
                role: agent.role.clone(),
            })?;
            Ok(())
        })?;
        let team = self.store.read(|c| agents::get_team(c, team.id))?;
        Ok((team, agent))
    }

    /// Creates a team with a Lead and workers (team mode).
    pub fn create_team(
        &self,
        project: ProjectId,
        name: &str,
        lead: &CreateAgent,
        workers: &[CreateAgent],
    ) -> Result<(agents::Team, Vec<agents::Agent>)> {
        let names: Vec<&str> = std::iter::once(lead)
            .chain(workers)
            .map(|a| a.name.as_str())
            .collect();
        self.check_names_free(&names)?;
        let team = agents::Team {
            id: TeamId::new(),
            name: name.to_owned(),
            template: self.manifest(&self.project(project)?)?.project.template,
            project: Some(project),
            lead: None,
            solo: false,
            created_at: now_iso(),
        };
        let lead_row = self.new_agent_row(team.id, lead, RoleKind::Lead);
        let worker_rows: Vec<_> = workers
            .iter()
            .map(|w| self.new_agent_row(team.id, w, RoleKind::Worker))
            .collect();
        self.store.commit(None, "user", |w| -> Result<()> {
            if !agents::teams_of_project(w.tx, project)?.is_empty() {
                return Err(OrchError::Conflict(
                    "the project already has a team (one team per project, decision 25)".into(),
                ));
            }
            agents::insert_team(w.tx, &team)?;
            w.emit(DomainEvent::TeamCreated {
                team: team.id,
                name: team.name.clone(),
                solo: false,
            })?;
            for a in std::iter::once(&lead_row).chain(worker_rows.iter()) {
                agents::insert_agent(w.tx, a)?;
                w.emit(DomainEvent::AgentCreated {
                    agent: a.id,
                    name: a.name.clone(),
                    team: Some(team.id),
                    role: a.role.clone(),
                })?;
            }
            agents::set_team_lead(w.tx, team.id, Some(lead_row.id))?;
            Ok(())
        })?;
        let mut all = vec![lead_row];
        all.extend(worker_rows);
        Ok((self.store.read(|c| agents::get_team(c, team.id))?, all))
    }

    /// Agent names are unique, ignoring case: an existing agent or another of `names` with the same name is refused.
    pub(crate) fn check_names_free(&self, names: &[&str]) -> Result<()> {
        let mut taken: HashSet<String> = self
            .agents()?
            .into_iter()
            .map(|a| a.name.to_lowercase())
            .collect();
        for n in names {
            if !taken.insert(n.trim().to_lowercase()) {
                return Err(OrchError::Conflict(format!(
                    "an agent named {:?} already exists; choose another name",
                    n.trim()
                )));
            }
        }
        Ok(())
    }

    fn new_agent_row(&self, team: TeamId, a: &CreateAgent, kind: RoleKind) -> agents::Agent {
        agents::Agent {
            id: AgentId::new(),
            name: a.name.trim().to_owned(),
            team: Some(team),
            role: a.role.clone(),
            role_kind: kind,
            tags: a.tags.clone(),
            status: AgentStatus::Idle,
            backend: BackendKind::Api,
            model: a.model.clone(),
            mcp: serde_json::json!([]),
            skills: vec![],
            created_at: now_iso(),
        }
    }

    pub fn agent(&self, id: AgentId) -> Result<agents::Agent> {
        Ok(self.store.read(|c| agents::get_agent(c, id))?)
    }

    pub fn agents(&self) -> Result<Vec<agents::Agent>> {
        Ok(self.store.read(agents::agents)?)
    }

    pub fn agent_named(&self, name: &str) -> Result<agents::Agent> {
        self.store
            .read(|c| agents::agent_by_name(c, name))?
            .ok_or_else(|| OrchError::NotFound(format!("agent {name}")))
    }

    /// Where the agent's vendor CLI keeps its session of each ticket between the ticket's runs (ADR-134), one folder per
    /// ticket key.
    pub fn cli_sessions(&self, id: AgentId) -> PathBuf {
        self.layout.agent_dir(&id).join("cli-sessions")
    }

    /// Ends the kept CLI sessions of a ticket (its worktree is gone: they belong to work that is over).
    pub(crate) fn forget_cli_sessions(&self, key: &str) {
        for a in self.agents().unwrap_or_default() {
            let _ = std::fs::remove_dir_all(self.cli_sessions(a.id).join(key));
        }
    }

    /// The agent's private memory database (opened once per process).
    pub fn agent_store(&self, id: AgentId, dim: usize, model_id: &str) -> Result<Arc<AgentStore>> {
        let mut m = self.agent_stores.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = m.get(&id) {
            return Ok(s.clone());
        }
        // ADR-50: exact search below `[memory] ann_threshold` facts, the ANN sidecar from there.
        let options = agenium_store::AgentOptions {
            ann_threshold: self.config().memory.ann_threshold,
            ..Default::default()
        };
        let s = Arc::new(AgentStore::open_with(
            &self.layout.agent_db(&id),
            dim,
            model_id,
            &self.layout.backups(),
            options,
        )?);
        m.insert(id, s.clone());
        Ok(s)
    }

    /// The shared pool of generalized lessons (ADR-87): a memory database under a reserved nil agent id that every
    /// agent also recalls from. Real agents get a random id, so the nil id never collides.
    pub fn pool_memory(&self) -> Result<Arc<AgentStore>> {
        let e = self.embedder();
        self.agent_store(AgentId::from_ulid(ulid::Ulid::nil()), e.dim(), e.model_id())
    }

    /// The index of a project's closed tickets' traces (ADR-158 decision 31), an agent database's layout under
    /// `<root>/db/projects/` (opened once per process).
    pub fn trace_store(&self, project: ProjectId) -> Result<Arc<AgentStore>> {
        let mut m = self.trace_stores.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = m.get(&project) {
            return Ok(s.clone());
        }
        let e = self.embedder();
        let dir = self.layout.db().join("projects");
        std::fs::create_dir_all(&dir)
            .map_err(|e| OrchError::Invalid(format!("{}: {e}", dir.display())))?;
        let s = Arc::new(AgentStore::open(
            &dir.join(format!("{project}-traces.db")),
            e.dim(),
            e.model_id(),
            &self.layout.backups(),
        )?);
        m.insert(project, s.clone());
        Ok(s)
    }

    /// Closes this process's handle of an agent database (deletion).
    pub(crate) fn forget_agent_store(&self, id: AgentId) {
        self.agent_stores
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&id);
    }

    /// Agent databases currently open in this process.
    pub fn open_agent_stores(&self) -> Vec<(AgentId, Arc<AgentStore>)> {
        self.agent_stores
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect()
    }

    /// The team working on a project and its members.
    pub fn team_of_project(
        &self,
        project: ProjectId,
    ) -> Result<Option<(agents::Team, Vec<agents::Agent>)>> {
        self.store
            .read(|c| {
                let Some(t) = agents::teams_of_project(c, project)?.into_iter().next() else {
                    return Ok(None);
                };
                let members = agents::agents_of_team(c, t.id)?;
                Ok(Some((t, members)))
            })
            .map_err(Into::into)
    }

    // ------------------------------------------------------------ queries

    pub fn ticket(&self, project: ProjectId, key: &TicketKey) -> Result<Ticket> {
        Ok(self.store.read(|c| tickets::get_by_key(c, project, key))?)
    }

    pub fn tickets(&self, project: ProjectId) -> Result<Vec<Ticket>> {
        Ok(self
            .store
            .read(|c| tickets::list(c, project, &Default::default()))?)
    }

    pub(crate) fn merge_lock(&self, project: ProjectId) -> Arc<Mutex<()>> {
        self.merge_locks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .entry(project)
            .or_default()
            .clone()
    }

    pub(crate) fn inject(&self, point: &str) -> Result<()> {
        let mut f = self.faults.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(i) = f.iter().position(|p| p == point) {
            f.remove(i);
            return Err(OrchError::Invalid(format!("injected fault at {point}")));
        }
        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Break the store -> sink(bus) link so the database closes promptly (ADR-45 note).
        self.store.set_sink(None);
    }
}

/// Stable id of a root (names the vault master key in the keyring). Stored in `<root>/root-id`.
fn root_id(layout: &RootLayout) -> Result<String> {
    let p = layout.root.join("root-id");
    match std::fs::read_to_string(&p) {
        Ok(s) if !s.trim().is_empty() => Ok(s.trim().to_owned()),
        _ => {
            let id = ulid::Ulid::new().to_string();
            agenium_config::write_atomic(&p, id.as_bytes())?;
            Ok(id)
        }
    }
}

/// Canonical absolute path without the Windows verbatim prefix (git and users expect `D:\x`, not `\\?\D:\x`).
pub fn canonical(p: &Path) -> PathBuf {
    let c = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let s = c.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => c,
    }
}
