//! The user's side of `propose_role` (decision 3l): approving creates the role when it is new (in the ROLE.md shape
//! of ADR-64), an agent with it, and adds the agent to the proposing Lead's team; declining keeps the reason for
//! the Lead. Nothing is created before the user decides.

use agenium_core::*;
use agenium_store::core::agents;
use agenium_store::core::roles::{self, ProposalStatus, RoleProposal};

use crate::app::App;
use crate::registry::{NewAgent, RoleDef};
use crate::{OrchError, Result};

/// ROLE.md of a role the Lead proposed and the user approved, from the proposal's text.
fn role_text(p: &agenium_core::forms::ProposeRole) -> String {
    format!(
        "## Mission\n{title}: {description}\n\n## Rules\n- Work only on the tickets assigned to you, inside the ticket worktree.\n- Ask the Lead (ask_question) when the ticket is unclear or needs something you do not have.\n\n## Deliverables\n- What each ticket asks for, with evidence for every acceptance criterion in the report.\n\n## Workflow\n1. Read the ticket, its comments and the requirements it serves.\n2. Do the work in small verified steps.\n3. Report with submit_report.\n\n## Done when\n- Every acceptance criterion is met and shown in the report.\n",
        title = p.title.trim(),
        description = p.description.trim(),
    )
}

impl App {
    pub fn role_proposals(&self, project: ProjectId) -> Result<Vec<RoleProposal>> {
        Ok(self.store.read(|c| roles::for_project(c, project))?)
    }

    /// The user approves a pending proposal (an agent with the role joins the team; `model` = the Lead's when
    /// `None`) or declines it with `note`. Returns the proposal as decided.
    pub fn decide_role_proposal(
        &self,
        project: ProjectId,
        id: &str,
        approve: bool,
        model: Option<(BackendKind, ModelBinding)>,
        note: Option<&str>,
    ) -> Result<RoleProposal> {
        let p = self
            .store
            .read(|c| roles::get(c, id))?
            .filter(|p| p.project == project)
            .ok_or_else(|| OrchError::NotFound(format!("role proposal {id}")))?;
        if p.status != ProposalStatus::Pending {
            return Err(OrchError::Conflict(format!(
                "role proposal {id} was already decided"
            )));
        }
        let mut agent = None;
        if approve {
            let role = p.body.role_id.clone();
            if self.role(&role).is_err() {
                self.put_role(&RoleDef {
                    id: role.clone(),
                    prompt: role_text(&p.body),
                    tags: p.body.tags.clone(),
                    allowed_paths: vec![],
                    planner: false,
                    lead: false,
                })?;
            }
            let lead = self.agent(p.proposed_by)?;
            let (backend, binding) = model.unwrap_or((lead.backend, lead.model.clone()));
            let library = self.skill_registry().library();
            let created = self.create_agent(&NewAgent {
                name: self.free_agent_name(&role)?,
                role: role.clone(),
                tags: (!p.body.tags.is_empty()).then(|| p.body.tags.clone()),
                model: binding,
                backend,
                skills: p
                    .body
                    .skills
                    .iter()
                    .filter(|s| library.get(s).is_some())
                    .cloned()
                    .collect(),
                mcp: vec![],
            })?;
            self.add_team_member(p.team, created.id)?;
            agent = Some(created.id);
        }
        self.store.commit(None, "user", |w| -> Result<()> {
            roles::decide(
                w.tx,
                id,
                if approve {
                    ProposalStatus::Approved
                } else {
                    ProposalStatus::Declined
                },
                agent,
                note,
                &now_iso(),
            )?;
            w.emit(DomainEvent::RoleProposalDecided {
                project,
                proposal: id.to_owned(),
                approved: approve,
                agent,
            })?;
            Ok(())
        })?;
        self.store
            .read(|c| roles::get(c, id))?
            .ok_or_else(|| OrchError::NotFound(format!("role proposal {id}")))
    }

    /// `base`, or `base-2`, `base-3`, ... when an agent already has that name (ignoring case).
    pub(crate) fn free_agent_name(&self, base: &str) -> Result<String> {
        let taken: std::collections::HashSet<String> = self
            .store
            .read(agents::agents)?
            .into_iter()
            .map(|a| a.name.to_lowercase())
            .collect();
        let mut name = base.to_owned();
        let mut n = 1;
        while taken.contains(&name.to_lowercase()) {
            n += 1;
            name = format!("{base}-{n}");
        }
        Ok(name)
    }
}
