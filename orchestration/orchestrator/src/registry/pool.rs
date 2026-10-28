//! The agent pool: agents created on their own, their role, model, skills and MCP servers.

use super::*;

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

pub(super) fn api() -> BackendKind {
    BackendKind::Api
}

impl App {
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

    pub(super) fn check_model(&self, backend: BackendKind, m: &ModelBinding) -> Result<()> {
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

    pub(super) fn check_skills(&self, skills: &[String]) -> Result<()> {
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

    pub(super) fn check_mcp(&self, mcp: &[agenium_mcp::Binding]) -> Result<()> {
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

    pub(super) fn pool_agent(
        &self,
        c: &rusqlite::Connection,
        id: AgentId,
    ) -> Result<agents::Agent> {
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
}
