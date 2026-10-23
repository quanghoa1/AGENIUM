//! The application service: owns the root, configuration, `core.db`, the realtime bus and the secret store, and
//! exposes every state-changing operation. UI/CLI/agent runtimes call only this.

use std::collections::HashMap;
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

use crate::manifest::{ProjectManifest, ProjectSection};
use crate::{OrchError, Result};

/// Builds the pre-merge check of a project. The runtime decides how commands run (sandbox or approval); tests
/// and projects without a check use [`NoCheckFactory`].
pub trait CheckFactory: Send + Sync {
    fn for_project(
        &self,
        project: &projects::Project,
        manifest: &ProjectManifest,
    ) -> Arc<dyn CheckRunner>;
}

pub struct NoCheckFactory;
impl CheckFactory for NoCheckFactory {
    fn for_project(&self, _: &projects::Project, _: &ProjectManifest) -> Arc<dyn CheckRunner> {
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
    agent_stores: Mutex<HashMap<AgentId, Arc<AgentStore>>>,
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
    /// Folder of the project; default `<root>/projects/<name>`. May be an existing repository.
    pub path: Option<PathBuf>,
    pub key_prefix: Option<String>,
    pub template: Option<String>,
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
            agent_stores: Mutex::new(HashMap::new()),
        };
        let report = app.reconcile()?;
        Ok((app, report))
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

    /// Creates a project: folder + standard layout + `agenium.toml` + git repository (`git init` when needed,
    /// initial commit), then registers it in `core.db`. An existing folder with `agenium.toml` is adopted.
    pub fn create_project(&self, req: &CreateProject) -> Result<projects::Project> {
        let name = req.name.trim();
        if name.is_empty() || name.contains(['/', '\\', ':']) {
            return Err(OrchError::Invalid(format!(
                "project name {name:?} must be non-empty and contain no path separators"
            )));
        }
        let path = req
            .path
            .clone()
            .unwrap_or_else(|| self.projects_dir().join(name));
        std::fs::create_dir_all(&path)
            .map_err(|e| OrchError::Invalid(format!("{}: {e}", path.display())))?;
        let path = canonical(&path);
        let manifest = match ProjectManifest::load(&path)? {
            Some(m) => m,
            None => {
                let m = ProjectManifest {
                    project: ProjectSection {
                        id: ProjectId::new().to_string(),
                        name: name.to_owned(),
                        key_prefix: req.key_prefix.clone().unwrap_or_else(|| "AGN".into()),
                        main_branch: "main".into(),
                        template: req.template.clone().unwrap_or_else(|| "coding@1".into()),
                    },
                    worktree: Default::default(),
                    checks: Default::default(),
                    network: Default::default(),
                };
                m.save(&path)?;
                m
            }
        };
        let id: ProjectId = manifest
            .project
            .id
            .parse()
            .map_err(|e| OrchError::Invalid(format!("agenium.toml: {e}")))?;
        for d in PROJECT_DIRS {
            let dir = path.join(d);
            std::fs::create_dir_all(&dir)
                .map_err(|e| OrchError::Invalid(format!("{}: {e}", dir.display())))?;
            let keep = dir.join(".gitkeep");
            if std::fs::read_dir(&dir)
                .map(|mut it| it.next().is_none())
                .unwrap_or(false)
            {
                std::fs::write(&keep, b"")
                    .map_err(|e| OrchError::Invalid(format!("{}: {e}", keep.display())))?;
            }
        }
        let repo = Repo::open(&path, &manifest.project.main_branch);
        repo.init_local()?;
        repo.commit_all(
            &path,
            "Agenium project layout",
            (
                agenium_vcs::git::COMMITTER_NAME,
                agenium_vcs::git::COMMITTER_EMAIL,
            ),
            &[],
        )?;

        let path_str = path.to_string_lossy().into_owned();
        if let Some(existing) = self.store.read(|c| projects::by_path(c, &path_str))? {
            return Ok(existing);
        }
        let np = projects::NewProject {
            id,
            name: manifest.project.name.clone(),
            path: path_str,
            key_prefix: manifest.project.key_prefix.clone(),
            main_branch: manifest.project.main_branch.clone(),
        };
        let p = self
            .store
            .commit(None, "user", |w| -> std::result::Result<_, StoreError> {
                let p = projects::insert(w.tx, &np)?;
                w.emit(DomainEvent::ProjectCreated {
                    project: p.id,
                    name: p.name.clone(),
                    path: p.path.clone(),
                })?;
                Ok(p)
            })?
            .value;
        Ok(p)
    }

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
        self.store
            .commit(None, "user", |w| -> std::result::Result<_, StoreError> {
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
                team: team.id,
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
                    team: team.id,
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

    fn new_agent_row(&self, team: TeamId, a: &CreateAgent, kind: RoleKind) -> agents::Agent {
        agents::Agent {
            id: AgentId::new(),
            name: a.name.trim().to_owned(),
            team,
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

    /// Changes an agent's model. Memory, MCP and skills belong to the agent and are untouched (decision 3b).
    pub fn set_agent_model(&self, id: AgentId, model: &ModelBinding) -> Result<()> {
        self.store
            .commit(None, "user", |w| -> std::result::Result<_, StoreError> {
                agents::set_agent_model(w.tx, id, model)?;
                w.emit(DomainEvent::AgentUpdated {
                    agent: id,
                    field: "model".into(),
                })?;
                Ok(())
            })?;
        Ok(())
    }

    /// Sets the agent's skill binding (empty = every skill of its role and its own).
    pub fn set_agent_skills(&self, id: AgentId, skills: &[String]) -> Result<()> {
        if let Some(bad) = skills.iter().find(|s| !agenium_skills::valid_name(s)) {
            return Err(OrchError::Invalid(format!("invalid skill name {bad:?}")));
        }
        self.store
            .commit(None, "user", |w| -> std::result::Result<_, StoreError> {
                agents::set_agent_skills(w.tx, id, skills)?;
                w.emit(DomainEvent::AgentUpdated {
                    agent: id,
                    field: "skills".into(),
                })?;
                Ok(())
            })?;
        Ok(())
    }

    pub fn agent_named(&self, name: &str) -> Result<agents::Agent> {
        self.store
            .read(|c| agents::agent_by_name(c, name))?
            .ok_or_else(|| OrchError::NotFound(format!("agent {name}")))
    }

    pub fn set_agent_status(&self, id: AgentId, to: AgentStatus) -> Result<()> {
        self.store
            .commit(None, "system", |w| -> std::result::Result<_, StoreError> {
                let from = agents::set_agent_status(w.tx, id, to)?;
                if from != to {
                    w.emit(DomainEvent::AgentStatusChanged {
                        agent: id,
                        from,
                        to,
                    })?;
                }
                Ok(())
            })?;
        Ok(())
    }

    /// The agent's private memory database (opened once per process).
    pub fn agent_store(&self, id: AgentId, dim: usize, model_id: &str) -> Result<Arc<AgentStore>> {
        let mut m = self.agent_stores.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = m.get(&id) {
            return Ok(s.clone());
        }
        let s = Arc::new(AgentStore::open(
            &self.layout.agent_db(&id),
            dim,
            model_id,
            &self.layout.backups(),
        )?);
        m.insert(id, s.clone());
        Ok(s)
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
