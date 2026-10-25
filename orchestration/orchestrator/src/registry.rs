//! Agent registry (ADR-61): agents are long-lived and created on their own into the pool (architecture page
//! "Agent registry": the user creates agents first), each with a role, its own model, the library skills and MCP
//! servers it enables; teams are then composed from pool agents (one Lead, workers, at most one planner). An agent
//! is in at most one team at a time (decision 3). The system only suggests a Lead; the user decides.
//!
//! Also here: roles (template presets and the user's own), the system skill library and model providers.

use std::path::Path;

use agenium_config::{ProviderConfig, ProviderKind};
use agenium_core::*;
use agenium_store::core::agents;
use agenium_store::StoreError;
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::team_templates::RoleFile;
use crate::{OrchError, Result};

fn invalid(msg: impl Into<String>) -> OrchError {
    OrchError::Invalid(msg.into())
}

fn io(e: impl std::fmt::Display) -> OrchError {
    OrchError::Invalid(e.to_string())
}

fn slug_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 60
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// A role an agent can take: a folder under `<root>/roles/<id>` or a template preset not written yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleInfo {
    pub id: String,
    /// ROLE.md text.
    pub prompt: String,
    pub tags: Vec<String>,
    pub allowed_paths: Vec<String>,
    pub planner: bool,
    /// Meant to lead a team (suggestion only).
    pub lead: bool,
    /// `<template>@<version>` for a template role, empty for the user's own.
    pub template: String,
    /// The folder exists (a preset is written when an agent first takes it).
    pub exists: bool,
    /// Live agents with this role.
    pub agents: usize,
    /// Advice about the ROLE.md text (sections, length, near-duplicate roles; ADR-64).
    pub lint: Vec<String>,
}

/// A role written by the user (or a preset edited before use).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleDef {
    pub id: String,
    pub prompt: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub planner: bool,
    #[serde(default)]
    pub lead: bool,
}

/// A new agent for the pool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewAgent {
    pub name: String,
    /// Role id (an existing role folder or a template preset).
    pub role: String,
    /// Capability tags; `None` = the role's tags.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// The model (an API provider id) or, with `backend = Cli`, the CLI program and its model.
    pub model: ModelBinding,
    #[serde(default = "api")]
    pub backend: BackendKind,
    /// Library skills the agent enables.
    #[serde(default)]
    pub skills: Vec<String>,
    /// MCP servers the agent enables (with its credential bindings).
    #[serde(default)]
    pub mcp: Vec<agenium_mcp::Binding>,
}

fn api() -> BackendKind {
    BackendKind::Api
}

/// A team composed from pool agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewTeam {
    pub name: String,
    /// Project to work on now (one team per project, decision 25); `None` = assigned later.
    #[serde(default)]
    pub project: Option<ProjectId>,
    pub lead: AgentId,
    /// Workers (the planner, if any, is one of them: its role says so).
    #[serde(default)]
    pub members: Vec<AgentId>,
    /// A Solo: one agent that is Lead and worker (`members` empty).
    #[serde(default)]
    pub solo: bool,
}

/// The suggested Lead of a set of agents and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LeadSuggestion {
    pub agent: AgentId,
    pub reasons: Vec<String>,
}

/// A provider as the user enters it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderInput {
    pub id: String,
    pub kind: ProviderKind,
    #[serde(default)]
    pub base_url: Option<String>,
    /// New API key (stored in the vault as `provider/<id>`); `None` keeps the stored one.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Drop the stored key (a local endpoint without a key).
    #[serde(default)]
    pub clear_key: bool,
    #[serde(default = "four")]
    pub max_concurrency: u32,
    /// Models listed by hand (endpoints that cannot list them).
    #[serde(default)]
    pub models: Vec<String>,
}

fn four() -> u32 {
    4
}

impl App {
    // ------------------------------------------------------------ roles

    /// Role folders, then the template roles not written yet (presets).
    pub fn roles(&self) -> Result<Vec<RoleInfo>> {
        let live = self.agents()?;
        let count = |id: &str| live.iter().filter(|a| a.role == id).count();
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.layout().roles()) {
            let mut dirs: Vec<_> = rd
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.starts_with('.'))
                .collect();
            dirs.sort();
            for id in dirs {
                let f = self.role_file(&id);
                let prompt =
                    std::fs::read_to_string(self.layout().roles().join(&id).join("ROLE.md"))
                        .unwrap_or_default();
                out.push(RoleInfo {
                    agents: count(&id),
                    id,
                    prompt: prompt.trim().to_owned(),
                    tags: f.tags,
                    allowed_paths: f.allowed_paths,
                    planner: f.planner,
                    lead: f.lead,
                    template: f.template,
                    exists: true,
                    lint: vec![],
                });
            }
        }
        for t in self.team_templates()? {
            for r in &t.roles {
                let id = t.role_dir(&r.id);
                if out.iter().any(|x| x.id == id) {
                    continue;
                }
                out.push(RoleInfo {
                    agents: count(&id),
                    id,
                    prompt: r.prompt.trim().to_owned(),
                    tags: r.tags.clone(),
                    allowed_paths: r.allowed_paths.clone(),
                    planner: r.planner,
                    lead: r.kind == RoleKind::Lead,
                    template: format!("{}@{}", t.id, t.version),
                    exists: false,
                    lint: vec![],
                });
            }
        }
        for r in crate::role_library::library() {
            if out.iter().any(|x| x.id == r.id) {
                continue;
            }
            out.push(RoleInfo {
                agents: count(&r.id),
                prompt: r.prompt.trim().to_owned(),
                tags: r.tags,
                allowed_paths: r.allowed_paths,
                planner: r.planner,
                lead: r.kind == RoleKind::Lead,
                template: crate::role_library::LIBRARY_ORIGIN.into(),
                exists: false,
                lint: vec![],
                id: r.id,
            });
        }
        let texts: Vec<(String, String)> = out
            .iter()
            .map(|r| (r.id.clone(), r.prompt.clone()))
            .collect();
        let others: Vec<(&str, &str)> = texts
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        for r in &mut out {
            r.lint = crate::role_library::lint_role(&r.id, &r.prompt, &others);
        }
        Ok(out)
    }

    pub fn role(&self, id: &str) -> Result<RoleInfo> {
        self.roles()?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| OrchError::NotFound(format!("role {id:?}")))
    }

    /// Writes (or rewrites) a role folder: ROLE.md and role.toml. The template origin of an existing role is kept.
    pub fn put_role(&self, def: &RoleDef) -> Result<RoleInfo> {
        if !slug_ok(&def.id) {
            return Err(invalid(format!(
                "role id {:?}: lowercase letters, digits and '-' (at most 60)",
                def.id
            )));
        }
        if def.prompt.trim().is_empty() {
            return Err(invalid("the role needs a description (ROLE.md)"));
        }
        if def.planner && def.lead {
            return Err(invalid("a planner role cannot lead (ADR-59)"));
        }
        for p in &def.allowed_paths {
            if p.contains("..") || Path::new(p).is_absolute() {
                return Err(invalid(format!(
                    "allowed path {p:?} must stay inside the worktree"
                )));
            }
        }
        let dir = self.layout().roles().join(&def.id);
        std::fs::create_dir_all(&dir).map_err(io)?;
        std::fs::write(dir.join("ROLE.md"), format!("{}\n", def.prompt.trim())).map_err(io)?;
        let old = self.role_file(&def.id);
        let file = RoleFile {
            template: old.template,
            allowed_paths: def.allowed_paths.clone(),
            planner: def.planner,
            lead: def.lead,
            tags: def.tags.clone(),
        };
        std::fs::write(
            dir.join("role.toml"),
            toml::to_string(&file).unwrap_or_default(),
        )
        .map_err(io)?;
        self.role(&def.id)
    }

    /// Deletes a role folder no live agent uses.
    pub fn remove_role(&self, id: &str) -> Result<()> {
        if !slug_ok(id) {
            return Err(invalid(format!("role id {id:?}")));
        }
        let users: Vec<String> = self
            .agents()?
            .into_iter()
            .filter(|a| a.role == id)
            .map(|a| a.name)
            .collect();
        if !users.is_empty() {
            return Err(OrchError::Conflict(format!(
                "agents still have role {id}: {}",
                users.join(", ")
            )));
        }
        let dir = self.layout().roles().join(id);
        if !dir.is_dir() {
            return Err(OrchError::NotFound(format!("role {id:?}")));
        }
        std::fs::remove_dir_all(dir).map_err(io)
    }

    /// Makes sure the role folder exists, writing a template preset when needed.
    fn ensure_role(&self, id: &str) -> Result<()> {
        if self.layout().roles().join(id).is_dir() {
            return Ok(());
        }
        for t in self.team_templates()? {
            if let Some(r) = t.roles.iter().find(|r| t.role_dir(&r.id) == id) {
                return self.materialize_role(&t, r);
            }
        }
        if let Some(r) = crate::role_library::library()
            .into_iter()
            .find(|r| r.id == id)
        {
            return self.write_role_folder(id, crate::role_library::LIBRARY_ORIGIN, &r);
        }
        Err(OrchError::NotFound(format!(
            "role {id:?}: create it first or pick a template role"
        )))
    }

    // ------------------------------------------------------------ agents

    /// Creates an agent in the pool (no team yet).
    pub fn create_agent(&self, a: &NewAgent) -> Result<agents::Agent> {
        let name = a.name.trim();
        if name.is_empty() || name.len() > 60 || name.contains('~') {
            return Err(invalid(
                "the agent name must be 1-60 characters and may not contain '~'",
            ));
        }
        self.check_names_free(&[name])?;
        self.ensure_role(&a.role)?;
        let role = self.role(&a.role)?;
        self.check_model(a.backend, &a.model)?;
        self.check_skills(&a.skills)?;
        self.check_mcp(&a.mcp)?;
        let row = agents::Agent {
            id: AgentId::new(),
            name: name.to_owned(),
            team: None,
            role: a.role.clone(),
            role_kind: RoleKind::Worker,
            tags: a.tags.clone().unwrap_or(role.tags),
            status: AgentStatus::Idle,
            backend: a.backend,
            model: a.model.clone(),
            mcp: serde_json::to_value(&a.mcp).map_err(io)?,
            skills: a.skills.clone(),
            created_at: now_iso(),
        };
        self.store()
            .commit(None, "user", |w| -> std::result::Result<(), StoreError> {
                agents::insert_agent(w.tx, &row)?;
                w.emit(DomainEvent::AgentCreated {
                    agent: row.id,
                    name: row.name.clone(),
                    team: None,
                    role: row.role.clone(),
                })?;
                Ok(())
            })?;
        self.agent(row.id)
    }

    /// Changes an agent's role and tags (not while it works).
    pub fn set_agent_role(&self, id: AgentId, role: &str, tags: Option<Vec<String>>) -> Result<()> {
        let a = self.agent(id)?;
        if a.status == AgentStatus::Working {
            return Err(OrchError::Conflict(format!(
                "{} is working; wait until its run ends",
                a.name
            )));
        }
        self.ensure_role(role)?;
        let info = self.role(role)?;
        if info.planner && a.role_kind == RoleKind::Lead {
            return Err(invalid(format!(
                "{} leads its team; a planner role cannot lead",
                a.name
            )));
        }
        let tags = tags.unwrap_or(info.tags);
        self.store()
            .commit(None, "user", |w| -> std::result::Result<(), StoreError> {
                agents::set_agent_role(w.tx, id, role, &tags)?;
                w.emit(DomainEvent::AgentUpdated {
                    agent: id,
                    field: "role".into(),
                })?;
                Ok(())
            })?;
        Ok(())
    }

    fn check_model(&self, backend: BackendKind, m: &ModelBinding) -> Result<()> {
        if m.model.trim().is_empty() && backend == BackendKind::Api {
            return Err(invalid("pick a model"));
        }
        match backend {
            BackendKind::Api => {
                if self.config().provider(&m.provider).is_none() {
                    return Err(invalid(format!(
                        "provider {:?} is not configured",
                        m.provider
                    )));
                }
            }
            BackendKind::Cli => {
                if !agenium_cli_backend::CliKind::ALL
                    .iter()
                    .any(|k| k.program() == m.provider)
                {
                    return Err(invalid(format!(
                        "{:?} is not a known agent CLI",
                        m.provider
                    )));
                }
            }
        }
        Ok(())
    }

    fn check_skills(&self, skills: &[String]) -> Result<()> {
        if skills.is_empty() {
            return Ok(());
        }
        let lib = self.skill_registry().library();
        for s in skills {
            if lib.get(s).is_none() {
                return Err(invalid(format!("skill {s:?} is not in the library")));
            }
        }
        Ok(())
    }

    fn check_mcp(&self, mcp: &[agenium_mcp::Binding]) -> Result<()> {
        let cfg = self.config();
        for b in mcp {
            if cfg.mcp_server(&b.server).is_none() {
                return Err(invalid(format!(
                    "MCP server {:?} is not configured",
                    b.server
                )));
            }
        }
        Ok(())
    }

    /// Turns one library skill on or off for an agent.
    pub fn toggle_agent_skill(&self, id: AgentId, skill: &str, on: bool) -> Result<Vec<String>> {
        let a = self.agent(id)?;
        let mut skills = a.skills.clone();
        skills.retain(|s| s != skill);
        if on {
            self.check_skills(&[skill.to_owned()])?;
            skills.push(skill.to_owned());
            skills.sort();
        }
        self.set_agent_skills(id, &skills)?;
        Ok(skills)
    }

    /// Turns one MCP server on (every tool, no credential binding) or off for an agent.
    pub fn toggle_agent_mcp(&self, id: AgentId, server: &str, on: bool) -> Result<()> {
        let a = self.agent(id)?;
        let mut b = agenium_mcp::parse_bindings(&a.mcp).map_err(io)?;
        let had = b.iter().any(|x| x.server == server);
        if on && !had {
            let new = agenium_mcp::Binding {
                server: server.to_owned(),
                credentials: Default::default(),
                keys: Default::default(),
                tools: None,
            };
            self.check_mcp(std::slice::from_ref(&new))?;
            b.push(new);
            b.sort_by(|x, y| x.server.cmp(&y.server));
        } else if !on {
            b.retain(|x| x.server != server);
        }
        self.set_agent_mcp(id, &b)
    }

    // ------------------------------------------------------------ teams

    fn pool_agent(&self, c: &rusqlite::Connection, id: AgentId) -> Result<agents::Agent> {
        let a =
            agents::find_agent(c, id)?.ok_or_else(|| OrchError::NotFound(format!("agent {id}")))?;
        if agents::is_deleted(c, id)? {
            return Err(OrchError::NotFound(format!("agent {id}")));
        }
        if a.team.is_some() {
            return Err(OrchError::Conflict(format!(
                "{} is already in a team (an agent is in one team at a time)",
                a.name
            )));
        }
        Ok(a)
    }

    /// Composes a team from pool agents.
    pub fn compose_team(&self, t: &NewTeam) -> Result<agents::Team> {
        let name = t.name.trim();
        if name.is_empty() {
            return Err(invalid("the team needs a name"));
        }
        if t.solo && !t.members.is_empty() {
            return Err(invalid("a Solo is one agent: no other members"));
        }
        if !t.solo && t.members.is_empty() {
            return Err(invalid("a team needs at least one worker besides the Lead"));
        }
        let mut seen = std::collections::BTreeSet::new();
        for id in std::iter::once(&t.lead).chain(&t.members) {
            if !seen.insert(*id) {
                return Err(invalid("an agent appears twice in the team"));
            }
        }
        let template = match t.project {
            Some(p) => self.manifest(&self.project(p)?)?.project.template,
            None => "custom".into(),
        };
        let team = agents::Team {
            id: TeamId::new(),
            name: name.to_owned(),
            template,
            project: t.project,
            lead: None,
            solo: t.solo,
            created_at: now_iso(),
        };
        let planner_of = |a: &agents::Agent| self.is_planner_role(&a.role);
        self.store().commit(None, "user", |w| -> Result<()> {
            let lead = self.pool_agent(w.tx, t.lead)?;
            if planner_of(&lead) {
                return Err(invalid(format!(
                    "{} has a planner role: a planner drafts plans and cannot lead",
                    lead.name
                )));
            }
            let mut planners = 0;
            for m in &t.members {
                let a = self.pool_agent(w.tx, *m)?;
                if planner_of(&a) {
                    planners += 1;
                }
            }
            if planners > 1 {
                return Err(invalid("a team has at most one planner"));
            }
            if let Some(p) = t.project {
                if !agents::teams_of_project(w.tx, p)?.is_empty() {
                    return Err(OrchError::Conflict(
                        "the project already has a team (one team per project, decision 25)".into(),
                    ));
                }
            }
            agents::insert_team(w.tx, &team)?;
            w.emit(DomainEvent::TeamCreated {
                team: team.id,
                name: team.name.clone(),
                solo: team.solo,
            })?;
            for (id, kind) in std::iter::once((t.lead, RoleKind::Lead))
                .chain(t.members.iter().map(|m| (*m, RoleKind::Worker)))
            {
                agents::set_agent_team(w.tx, id, Some(team.id))?;
                agents::set_agent_role_kind(w.tx, id, kind)?;
                w.emit(DomainEvent::AgentTeamChanged {
                    agent: id,
                    team: Some(team.id),
                })?;
            }
            agents::set_team_lead(w.tx, team.id, Some(t.lead))?;
            Ok(())
        })?;
        Ok(self.store().read(|c| agents::get_team(c, team.id))?)
    }

    pub fn team(&self, id: TeamId) -> Result<(agents::Team, Vec<agents::Agent>)> {
        Ok(self.store().read(|c| {
            Ok::<_, StoreError>((agents::get_team(c, id)?, agents::agents_of_team(c, id)?))
        })?)
    }

    pub fn teams(&self) -> Result<Vec<(agents::Team, Vec<agents::Agent>)>> {
        Ok(self.store().read(|c| {
            agents::teams(c)?
                .into_iter()
                .map(|t| {
                    let m = agents::agents_of_team(c, t.id)?;
                    Ok::<_, StoreError>((t, m))
                })
                .collect::<std::result::Result<Vec<_>, _>>()
        })?)
    }

    /// Open tickets of the team's project held by `agent` (or by anyone when `None`).
    fn open_tickets(&self, team: &agents::Team, agent: Option<AgentId>) -> Result<Vec<String>> {
        let Some(p) = team.project else {
            return Ok(vec![]);
        };
        Ok(self
            .tickets(p)?
            .into_iter()
            .filter(|t| !t.status.is_terminal() && (agent.is_none() || t.assignee == agent))
            .map(|t| t.key.to_string())
            .collect())
    }

    /// Adds a pool agent to a team as a worker.
    pub fn add_team_member(&self, team: TeamId, agent: AgentId) -> Result<()> {
        let (t, members) = self.team(team)?;
        if t.solo {
            return Err(invalid("a Solo has one agent; compose a team instead"));
        }
        self.store().commit(None, "user", |w| -> Result<()> {
            let a = self.pool_agent(w.tx, agent)?;
            if self.is_planner_role(&a.role)
                && members.iter().any(|m| self.is_planner_role(&m.role))
            {
                return Err(invalid("the team already has a planner"));
            }
            agents::set_agent_team(w.tx, agent, Some(team))?;
            agents::set_agent_role_kind(w.tx, agent, RoleKind::Worker)?;
            w.emit(DomainEvent::AgentTeamChanged {
                agent,
                team: Some(team),
            })?;
            Ok(())
        })?;
        Ok(())
    }

    /// Sends a worker back to the pool (not the Lead, not while it holds open tickets or works).
    pub fn remove_team_member(&self, team: TeamId, agent: AgentId) -> Result<()> {
        let (t, members) = self.team(team)?;
        let a = members
            .iter()
            .find(|m| m.id == agent)
            .ok_or_else(|| invalid("the agent is not a member of this team"))?;
        if t.lead == Some(agent) {
            return Err(invalid(
                "the Lead stays: make another member Lead first, or dissolve the team",
            ));
        }
        if a.status == AgentStatus::Working {
            return Err(OrchError::Conflict(format!(
                "{} is working; wait until its run ends",
                a.name
            )));
        }
        let open = self.open_tickets(&t, Some(agent))?;
        if !open.is_empty() {
            return Err(OrchError::Conflict(format!(
                "{} still has open tickets ({}): reassign them first",
                a.name,
                open.join(", ")
            )));
        }
        if members.len() <= 2 && !t.solo {
            return Err(invalid(
                "a team keeps at least one worker: add another first, or dissolve the team",
            ));
        }
        self.store()
            .commit(None, "user", |w| -> std::result::Result<(), StoreError> {
                agents::set_agent_team(w.tx, agent, None)?;
                agents::set_agent_role_kind(w.tx, agent, RoleKind::Worker)?;
                w.emit(DomainEvent::AgentTeamChanged { agent, team: None })?;
                Ok(())
            })?;
        Ok(())
    }

    /// Makes a member the team's Lead (the previous Lead becomes a worker). See also [`App::set_team_lead`].
    pub fn set_lead_of_team(&self, team: TeamId, agent: AgentId) -> Result<()> {
        let (t, _) = self.team(team)?;
        match t.project {
            Some(p) => self.set_team_lead(p, agent),
            None => {
                let (_, members) = self.team(team)?;
                let new = members
                    .iter()
                    .find(|m| m.id == agent)
                    .ok_or_else(|| invalid("the agent is not a member of this team"))?;
                if t.solo {
                    return Err(invalid("a Solo agent is its own Lead"));
                }
                if self.is_planner_role(&new.role) {
                    return Err(invalid(format!("{} is the team's planner", new.name)));
                }
                self.store()
                    .commit(None, "user", |w| -> std::result::Result<(), StoreError> {
                        if let Some(old) = t.lead {
                            agents::set_agent_role_kind(w.tx, old, RoleKind::Worker)?;
                        }
                        agents::set_agent_role_kind(w.tx, agent, RoleKind::Lead)?;
                        agents::set_team_lead(w.tx, team, Some(agent))?;
                        w.emit(DomainEvent::AgentUpdated {
                            agent,
                            field: "role_kind".into(),
                        })?;
                        Ok(())
                    })?;
                Ok(())
            }
        }
    }

    /// Points a team at a project (or frees it). Decision 25: one team per project, and a team leaves a project only
    /// when nothing is open there.
    pub fn set_team_project(&self, team: TeamId, project: Option<ProjectId>) -> Result<()> {
        let (t, _) = self.team(team)?;
        if t.project == project {
            return Ok(());
        }
        let open = self.open_tickets(&t, None)?;
        if !open.is_empty() {
            return Err(OrchError::Conflict(format!(
                "the team still has open tickets ({}) in its project",
                open.join(", ")
            )));
        }
        self.store().commit(None, "user", |w| -> Result<()> {
            if let Some(p) = project {
                if agents::teams_of_project(w.tx, p)?
                    .iter()
                    .any(|x| x.id != team)
                {
                    return Err(OrchError::Conflict(
                        "the project already has a team (one team per project, decision 25)".into(),
                    ));
                }
            }
            agents::set_team_project(w.tx, team, project)?;
            w.emit(DomainEvent::TeamProjectChanged { team, project })?;
            Ok(())
        })?;
        Ok(())
    }

    /// Dissolves a team: every member goes back to the pool. Refused while its project has open tickets.
    pub fn dissolve_team(&self, team: TeamId) -> Result<()> {
        let (t, members) = self.team(team)?;
        let open = self.open_tickets(&t, None)?;
        if !open.is_empty() {
            return Err(OrchError::Conflict(format!(
                "the team still has open tickets ({}); close or cancel them first",
                open.join(", ")
            )));
        }
        if let Some(a) = members.iter().find(|m| m.status == AgentStatus::Working) {
            return Err(OrchError::Conflict(format!(
                "{} is working; wait until its run ends",
                a.name
            )));
        }
        self.store()
            .commit(None, "user", |w| -> std::result::Result<(), StoreError> {
                agents::set_team_lead(w.tx, team, None)?;
                for m in &members {
                    agents::set_agent_team(w.tx, m.id, None)?;
                    agents::set_agent_role_kind(w.tx, m.id, RoleKind::Worker)?;
                    w.emit(DomainEvent::AgentTeamChanged {
                        agent: m.id,
                        team: None,
                    })?;
                }
                // Tombstoned former members still point at the team: free them too.
                w.tx.execute(
                    "UPDATE agent SET team = NULL WHERE team = ?1",
                    [team.to_string()],
                )?;
                agents::delete_team(w.tx, team)?;
                w.emit(DomainEvent::TeamDissolved {
                    team,
                    name: t.name.clone(),
                })?;
                Ok(())
            })?;
        Ok(())
    }

    /// Suggests a Lead among `candidates`: a role meant to lead, a role named lead, lead/planning/review tags, an
    /// enabled skill about leading a team. `None` when nothing points to one (the user picks).
    pub fn suggest_lead(&self, candidates: &[AgentId]) -> Result<Option<LeadSuggestion>> {
        let mut best: Option<(i32, LeadSuggestion)> = None;
        for id in candidates {
            let a = self.agent(*id)?;
            if self.is_planner_role(&a.role) {
                continue;
            }
            let mut score = 0;
            let mut reasons = Vec::new();
            let role = self.role_file(&a.role);
            if role.lead {
                score += 4;
                reasons.push(format!("role {} is meant to lead", a.role));
            } else if a.role == "lead" || a.role.ends_with("-lead") {
                score += 3;
                reasons.push(format!("role {}", a.role));
            }
            for tag in ["lead", "planning", "review"] {
                if a.tags.iter().any(|t| t == tag) {
                    score += 1;
                    reasons.push(format!("tag {tag}"));
                }
            }
            let set = self
                .skill_registry()
                .for_agent(&a.id.to_string(), &a.role, &a.skills);
            for s in &set.skills {
                let text = format!("{} {}", s.meta.name, s.meta.description).to_lowercase();
                if text.contains("lead") || text.contains("team management") {
                    score += 2;
                    reasons.push(format!("skill {}", s.meta.name));
                }
            }
            if score > 0 && best.as_ref().is_none_or(|(b, _)| score > *b) {
                best = Some((
                    score,
                    LeadSuggestion {
                        agent: a.id,
                        reasons,
                    },
                ));
            }
        }
        Ok(best.map(|(_, s)| s))
    }

    // ------------------------------------------------------------ skill library

    /// Imports a skill (folder, `.zip`) into the system library.
    pub fn import_library_skill(
        &self,
        src: &Path,
        replace: bool,
    ) -> Result<agenium_skills::SkillMeta> {
        self.skill_registry()
            .import(src, &agenium_skills::Owner::Library, replace)
            .map_err(io)
    }

    /// Removes a library skill. Agents that enable it lose it when `force`; otherwise refused while in use.
    pub fn remove_library_skill(&self, name: &str, force: bool) -> Result<()> {
        let users: Vec<agents::Agent> = self
            .agents()?
            .into_iter()
            .filter(|a| a.skills.iter().any(|s| s == name))
            .collect();
        if !users.is_empty() && !force {
            return Err(OrchError::Conflict(format!(
                "agents use skill {name}: {}",
                users
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        self.skill_registry()
            .remove(&agenium_skills::Owner::Library, name)
            .map_err(io)?;
        for a in users {
            let rest: Vec<String> = a.skills.into_iter().filter(|s| s != name).collect();
            self.set_agent_skills(a.id, &rest)?;
        }
        Ok(())
    }

    /// Removes an MCP server; with `force` the agents that enable it lose it first.
    pub fn remove_mcp_server_force(&self, id: &str) -> Result<()> {
        for a in self.agents()? {
            let b = agenium_mcp::parse_bindings(&a.mcp).map_err(io)?;
            if b.iter().any(|x| x.server == id) {
                let rest: Vec<_> = b.into_iter().filter(|x| x.server != id).collect();
                self.set_agent_mcp(a.id, &rest)?;
            }
        }
        self.remove_mcp_server(id)
    }

    // ------------------------------------------------------------ providers

    /// Adds or replaces a provider. A new key goes to the vault (`provider/<id>`), never to config.toml.
    pub fn put_provider(&self, p: &ProviderInput) -> Result<ProviderConfig> {
        if !slug_ok(&p.id) {
            return Err(invalid(format!(
                "provider id {:?}: lowercase letters, digits and '-'",
                p.id
            )));
        }
        if p.kind == ProviderKind::Mock {
            return Err(invalid("mock providers are for tests"));
        }
        if let Some(u) = &p.base_url {
            if !(u.starts_with("http://") || u.starts_with("https://")) {
                return Err(invalid(format!(
                    "base URL {u:?} must start with http:// or https://"
                )));
            }
        }
        if p.kind == ProviderKind::OpenaiCompat && p.base_url.as_deref().unwrap_or("").is_empty() {
            return Err(invalid("an OpenAI-compatible provider needs a base URL"));
        }
        let secret = format!("provider/{}", p.id);
        let old = self.config().provider(&p.id).cloned();
        let key_ref = match (&p.api_key, p.clear_key) {
            (Some(k), _) if !k.trim().is_empty() => {
                self.secrets().set(&secret, k.trim().as_bytes())?;
                secret.clone()
            }
            (_, true) => {
                let _ = self.secrets().delete(&secret);
                String::new()
            }
            _ => old
                .as_ref()
                .map(|o| o.api_key_ref.clone())
                .unwrap_or_default(),
        };
        if key_ref.is_empty() && matches!(p.kind, ProviderKind::Anthropic | ProviderKind::Gemini) {
            return Err(invalid("this provider needs an API key"));
        }
        let pc = ProviderConfig {
            id: p.id.clone(),
            kind: p.kind,
            base_url: p.base_url.clone().filter(|u| !u.trim().is_empty()),
            api_key_ref: key_ref,
            max_concurrency: p.max_concurrency.max(1),
            models: p.models.clone(),
            model_caps: old.map(|o| o.model_caps).unwrap_or_default(),
        };
        self.update_config(|c| {
            c.providers.retain(|x| x.id != pc.id);
            c.providers.push(pc.clone());
            c.providers.sort_by(|a, b| a.id.cmp(&b.id));
        })?;
        Ok(pc)
    }

    /// Removes a provider no agent (and no CLI fallback) uses, with its key.
    pub fn remove_provider(&self, id: &str) -> Result<()> {
        let cfg = self.config();
        let pc = cfg
            .provider(id)
            .cloned()
            .ok_or_else(|| OrchError::NotFound(format!("provider {id:?}")))?;
        let users: Vec<String> = self
            .agents()?
            .into_iter()
            .filter(|a| a.backend == BackendKind::Api && a.model.provider == id)
            .map(|a| a.name)
            .collect();
        if !users.is_empty() {
            return Err(OrchError::Conflict(format!(
                "agents use provider {id}: {}; switch their model first",
                users.join(", ")
            )));
        }
        if cfg.cli.fallback_provider == id {
            return Err(OrchError::Conflict(format!(
                "provider {id} is the CLI fallback; change it first"
            )));
        }
        self.update_config(|c| c.providers.retain(|x| x.id != id))?;
        if !pc.api_key_ref.is_empty() {
            let _ = self.secrets().delete(&pc.api_key_ref);
        }
        Ok(())
    }
}
