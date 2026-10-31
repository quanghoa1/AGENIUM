//! The user's side of an agent's new skill (ADR-91 amended by ADR-152): the proposal waits in Needs you; the user opens
//! it in the skill editor, changes it if they like and saves it into the library — the proposing agent gets it switched
//! on, every other agent may enable it — or declines it. A pending proposal whose lesson stops holding is withdrawn.

use agenium_core::*;
use agenium_store::core::roles::ProposalStatus;
use agenium_store::core::skill_proposals::{self, SkillProposal};

use crate::app::App;
use crate::{OrchError, Result};

impl App {
    /// The proposals waiting for the user, of `project` (or of every project).
    pub fn skill_proposals(&self, project: Option<ProjectId>) -> Result<Vec<SkillProposal>> {
        Ok(self
            .store()
            .read(|c| skill_proposals::pending(c, project))?)
    }

    pub fn skill_proposal(&self, id: &str) -> Result<SkillProposal> {
        self.store()
            .read(|c| skill_proposals::get(c, id))?
            .ok_or_else(|| OrchError::NotFound(format!("skill proposal {id}")))
    }

    /// The user's decision. Approving names the library skill the user saved from the proposal (in the skill editor,
    /// as it was or changed): it is switched on for the proposing agent. Declining keeps `note`.
    pub fn decide_skill_proposal(
        &self,
        id: &str,
        approve: bool,
        saved_as: Option<&str>,
        note: Option<&str>,
    ) -> Result<SkillProposal> {
        let p = self.skill_proposal(id)?;
        if p.status != ProposalStatus::Pending {
            return Err(OrchError::Conflict(format!(
                "skill proposal {id} was already decided"
            )));
        }
        let skill = if approve {
            let name = saved_as
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .ok_or_else(|| {
                    OrchError::Invalid("save the skill first, then approve with its name".into())
                })?;
            if self.skill_registry().library().get(name).is_none() {
                return Err(OrchError::Invalid(format!(
                    "{name} is not in the skill library: save it there first"
                )));
            }
            self.toggle_agent_skill(p.agent, name, true)?;
            Some(name.to_owned())
        } else {
            None
        };
        let status = if approve {
            ProposalStatus::Approved
        } else {
            ProposalStatus::Declined
        };
        self.store().commit(None, "user", |w| {
            skill_proposals::decide(w.tx, id, status, skill.as_deref(), note, &now_iso())?;
            w.emit(DomainEvent::AgentUpdated {
                agent: p.agent,
                field: format!(
                    "skill_proposal_{}:{}",
                    if approve { "saved" } else { "declined" },
                    skill.as_deref().unwrap_or(&p.name)
                ),
            })
            .map_err(OrchError::from)
        })?;
        self.skill_proposal(id)
    }

    /// Withdraws `agent`'s pending proposals whose lesson no longer holds (retired, deleted, or below the confidence
    /// a skill needs): declined with why, so the user never saves a skill on a lesson that turned out wrong. Returns the
    /// names withdrawn.
    pub fn withdraw_doubtful_proposals(&self, agent: AgentId) -> Result<Vec<String>> {
        let mem = self.agent_memory_store(agent)?;
        let mut out = Vec::new();
        for p in self
            .store()
            .read(|c| skill_proposals::of_agent(c, agent))?
            .into_iter()
            .filter(|p| p.status == ProposalStatus::Pending)
        {
            let why = match crate::skill_create::live_end(&mem, p.lesson) {
                None => Some("its lesson was retired or deleted"),
                Some(f) if f.confidence < crate::skill_create::DOUBTFUL_CONFIDENCE => {
                    Some("its lesson lost confidence after reopens")
                }
                Some(_) => None,
            };
            if let Some(w) = why {
                self.store().commit(None, "system", |tx| {
                    skill_proposals::decide(
                        tx.tx,
                        &p.id,
                        ProposalStatus::Declined,
                        None,
                        Some(w),
                        &now_iso(),
                    )
                    .map_err(OrchError::from)
                })?;
                out.push(format!("{}: {w}", p.name));
            }
        }
        Ok(out)
    }
}
