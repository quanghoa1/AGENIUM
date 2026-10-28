//! Teams composed from pool agents: members, the Lead, the project, dissolving, the suggested Lead.

use super::*;

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

impl App {
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
    pub(super) fn open_tickets(
        &self,
        team: &agents::Team,
        agent: Option<AgentId>,
    ) -> Result<Vec<String>> {
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
        if let Some(p) = project {
            if crate::project_close::closed(&self.project(p)?.settings) {
                return Err(OrchError::Conflict(
                    "the project is closed: reopen it first (it can take its team again then)"
                        .into(),
                ));
            }
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
            // A project given a team waits for no other (ADR-130).
            if let Some(p) = project {
                let mut s = agenium_store::core::projects::get(w.tx, p)?.settings;
                if let Some(o) = s.as_object_mut() {
                    if o.remove(crate::project_close::WAITS_FOR).is_some() {
                        agenium_store::core::projects::set_settings(w.tx, p, &s)?;
                    }
                }
            }
            Ok(())
        })?;
        if project.is_none() {
            self.hand_over(team)?;
        }
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
}
