//! An agent's settings the user changes: its name, model, backend (API or a vendor CLI), skills and status. A new model
//! frees an agent set aside for its old one (ADR-131/132).

use agenium_core::*;
use agenium_store::core::agents;
use agenium_store::StoreError;

use crate::app::App;
use crate::{OrchError, Result};

impl App {
    /// Renames an agent. The name is unique ignoring case (another agent's name is refused); the id, memory and
    /// history stay.
    pub fn rename_agent(&self, id: AgentId, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() || name.len() > 60 || name.contains('~') {
            return Err(OrchError::Invalid(
                "the agent name must be 1-60 characters and may not contain '~'".into(),
            ));
        }
        if self
            .agents()?
            .iter()
            .any(|a| a.id != id && a.name.to_lowercase() == name.to_lowercase())
        {
            return Err(OrchError::Conflict(format!(
                "an agent named {name:?} already exists; choose another name"
            )));
        }
        self.store
            .commit(None, "user", |w| -> std::result::Result<_, StoreError> {
                agents::set_agent_name(w.tx, id, name)?;
                w.emit(DomainEvent::AgentUpdated {
                    agent: id,
                    field: "name".into(),
                })?;
                Ok(())
            })?;
        Ok(())
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
        self.free_after_model_change(id)
    }

    /// An agent set aside for its model (quota used up, provider or login gone) goes back to work with a new one, which
    /// has its own quota and login. The ticket note said "change the model to continue", but the agent stayed set
    /// aside until it was also set back to Idle by hand.
    fn free_after_model_change(&self, id: AgentId) -> Result<()> {
        if !self.agent(id)?.status.assignable() {
            self.set_agent_status(id, AgentStatus::Idle)?;
        }
        Ok(())
    }

    /// Switches an agent between the API and a vendor CLI (decision 3c). With `Cli`, `model.provider` names the CLI
    /// (`devin`, ...) and `model.model` the CLI's model (empty = the CLI's default).
    pub fn set_agent_backend(
        &self,
        id: AgentId,
        backend: BackendKind,
        model: &ModelBinding,
    ) -> Result<()> {
        if backend == BackendKind::Cli
            && !agenium_cli_backend::CliKind::ALL
                .iter()
                .any(|k| k.program() == model.provider)
        {
            return Err(OrchError::Invalid(format!(
                "{:?} is not a known agent CLI",
                model.provider
            )));
        }
        self.store
            .commit(None, "user", |w| -> std::result::Result<_, StoreError> {
                agents::set_agent_backend(w.tx, id, backend)?;
                agents::set_agent_model(w.tx, id, model)?;
                w.emit(DomainEvent::AgentUpdated {
                    agent: id,
                    field: "backend".into(),
                })?;
                Ok(())
            })?;
        self.free_after_model_change(id)
    }

    /// Sets the agent's skill binding: the library skills it enables (its role's and its own skills are always
    /// on; ADR-61).
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
}
