//! The team: role proposals, the Lead, the team view and members' MCP keys. Split out of `engine.rs`.

use super::*;

impl App {
    /// A Lead proposes a role its team lacks (decision 3l): kept as a pending proposal; an agent is created and
    /// joins the team only when the user approves (`App::decide_role_proposal`).
    pub(super) fn propose_role(
        &self,
        ctx: &CallCtx,
        r: agenium_core::forms::ProposeRole,
    ) -> Result<Outcome> {
        use agenium_store::core::roles::{self, ProposalStatus, RoleProposal};
        let project = ctx.project;
        let Actor::Lead(lead) = ctx.actor else {
            return Err(denied("only a team's Lead proposes roles"));
        };
        let role = r.role_id.trim().to_owned();
        // The same rule `put_role` applies at approval, so an accepted proposal can be approved.
        if !agenium_core::validate::slug_ok(&role, crate::registry::ROLE_ID_MAX) {
            return Err(invalid(format!(
                "role_id {role:?}: lowercase letters, digits and '-', not at either end (at most 60)"
            )));
        }
        if r.title.trim().is_empty() || r.description.trim().len() < 20 {
            return Err(invalid(
                "a role proposal needs a title and a description of what the role does and why the team needs it",
            ));
        }
        self.commit(ctx, |w| {
            let team = require_lead(w, project, &ctx.actor)?
                .ok_or_else(|| denied("the project has no team"))?;
            if team.solo {
                return Err(denied(
                    "a Solo agent works alone: ask the user to build a team instead",
                ));
            }
            if let Some(m) = agents::agents_of_team(w.tx, team.id)?
                .into_iter()
                .find(|m| m.role == role)
            {
                return Err(invalid(format!(
                    "{} already has the role {role}: give the work to them",
                    m.name
                )));
            }
            if let Some(p) = roles::for_project(w.tx, project)?
                .into_iter()
                .find(|p| p.status == ProposalStatus::Pending && p.body.role_id == role)
            {
                return Err(invalid(format!(
                    "role {role} is already proposed ({}) and waits for the user",
                    p.id
                )));
            }
            let id = format!("RP-{}", ulid::Ulid::new());
            roles::insert(
                w.tx,
                &RoleProposal {
                    id: id.clone(),
                    project,
                    team: team.id,
                    proposed_by: lead,
                    body: agenium_core::forms::ProposeRole {
                        role_id: role.clone(),
                        ..r.clone()
                    },
                    status: ProposalStatus::Pending,
                    agent: None,
                    note: None,
                    created_at: now_iso(),
                    decided_at: None,
                },
            )?;
            w.emit(DomainEvent::RoleProposed {
                project,
                proposal: id.clone(),
                role: role.clone(),
            })?;
            w.emit(DomainEvent::NeedsUser {
                ticket: None,
                reason: format!("the Lead proposes a new role {role}: {}", r.title.trim()),
            })?;
            Ok(Outcome::ok(serde_json::json!({
                "status": "waiting_for_user_approval",
                "proposal": id,
                "next": "the user decides; on approval an agent with this role joins your team (get_team shows it). Until then keep its plan items unassigned.",
            })))
        })
    }

    /// The user puts another member in the Lead slot (architecture page: "Người dùng gán agent khác vào slot Lead").
    /// The new Lead must be a worker of the team (not the planner) without open tickets; the old Lead becomes a
    /// worker. Everything else (inbox, plan, tickets) lives in the database and carries over; the chat history of the
    /// old Lead stays in its own memory.
    pub fn set_team_lead(&self, project: ProjectId, agent: AgentId) -> Result<()> {
        let (team, members) = self
            .team_of_project(project)?
            .ok_or_else(|| invalid("the project has no team"))?;
        if team.solo {
            return Err(invalid("a Solo agent is its own Lead"));
        }
        let new = members
            .iter()
            .find(|m| m.id == agent)
            .ok_or_else(|| invalid("the agent is not a member of this team"))?;
        if team.lead == Some(agent) {
            return Ok(());
        }
        if self.is_planner_role(&new.role) {
            return Err(invalid(format!("{} is the team's planner", new.name)));
        }
        let open: Vec<String> = self
            .tickets(project)?
            .into_iter()
            .filter(|t| t.assignee == Some(agent) && !t.status.is_terminal())
            .map(|t| t.key.to_string())
            .collect();
        if !open.is_empty() {
            return Err(invalid(format!(
                "{} still has open tickets ({}): reassign them first",
                new.name,
                open.join(", ")
            )));
        }
        if new.status == AgentStatus::Working {
            return Err(invalid(format!(
                "{} is working; wait until its run ends",
                new.name
            )));
        }
        self.store
            .commit(None, "user", |w| -> Result<()> {
                if let Some(old) = team.lead {
                    agents::set_agent_role_kind(w.tx, old, RoleKind::Worker)?;
                    w.emit(DomainEvent::AgentUpdated {
                        agent: old,
                        field: "role_kind".into(),
                    })?;
                }
                agents::set_agent_role_kind(w.tx, agent, RoleKind::Lead)?;
                agents::set_team_lead(w.tx, team.id, Some(agent))?;
                w.emit(DomainEvent::AgentUpdated {
                    agent,
                    field: "role_kind".into(),
                })?;
                Ok(())
            })
            .map(|c| c.value)
    }

    pub(super) fn get_team(&self, ctx: &CallCtx) -> Result<Outcome> {
        let (team, members) = self
            .team_of_project(ctx.project)?
            .ok_or_else(|| invalid("the project has no team"))?;
        if !matches!(ctx.actor, Actor::Lead(a) if team.lead == Some(a)) && ctx.actor != Actor::User
        {
            return Err(denied("only the Lead sees the team roster"));
        }
        let open: HashMap<AgentId, TicketKey> = self
            .tickets(ctx.project)?
            .into_iter()
            .filter(|t| matches!(t.status, TicketStatus::InProgress | TicketStatus::Pending))
            .filter_map(|t| t.assignee.map(|a| (a, t.key)))
            .collect();
        let roster: Vec<serde_json::Value> = members
            .iter()
            .map(|m| {
                // Every MCP server the member uses, with its key names and whether the member has its own key
                // (values never); a server without keys is listed too.
                let mut mcp: Vec<serde_json::Value> = agenium_mcp::parse_bindings(&m.mcp)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|b| serde_json::json!({ "server": b.server, "keys": [] }))
                    .collect();
                for k in self.agent_mcp_keys(m.id).unwrap_or_default() {
                    let entry = serde_json::json!({ "key": k.name, "own_key": k.own.is_some() });
                    match mcp.iter_mut().find(|s| s["server"] == k.server.as_str()) {
                        Some(s) => s["keys"].as_array_mut().unwrap().push(entry),
                        None => mcp.push(serde_json::json!({ "server": k.server, "keys": [entry] })),
                    }
                }
                serde_json::json!({
                    "agent": m.id, "name": m.name, "role": m.role, "kind": m.role_kind, "tags": m.tags,
                    "model": format!("{}/{}", m.model.provider, m.model.model), "status": m.status,
                    "current_ticket": open.get(&m.id), "planner": self.is_planner_role(&m.role), "mcp": mcp,
                    // Where the member may write: a ticket's paths must fall inside (empty: anywhere).
                    "allowed_paths": self.role_allowed_paths(&m.role),
                })
            })
            .collect();
        // Specialist roles the Lead can propose_role by id when the team lacks one (decision 3l).
        let library: Vec<serde_json::Value> = if team.solo {
            vec![]
        } else {
            crate::role_library::library()
                .into_iter()
                .map(|r| serde_json::json!({ "role": r.id, "mission": crate::role_library::mission(&r.prompt) }))
                .collect()
        };
        Ok(Outcome::ok(serde_json::json!({
            "team": team.name, "solo": team.solo, "members": roster, "role_library": library,
        })))
    }

    /// The Lead gives a member of its team (or itself) its own key for an MCP server the member uses.
    pub(super) fn set_member_mcp_key(&self, ctx: &CallCtx, k: SetMemberMcpKey) -> Result<Outcome> {
        let (team, members) = self
            .team_of_project(ctx.project)?
            .ok_or_else(|| invalid("the project has no team"))?;
        if !matches!(ctx.actor, Actor::Lead(a) if team.lead == Some(a)) && ctx.actor != Actor::User
        {
            return Err(denied("only the Lead sets the keys of its team"));
        }
        let m = members
            .iter()
            .find(|m| m.id == k.agent)
            .ok_or_else(|| denied(format!("{} is not a member of your team", k.agent)))?;
        self.set_agent_mcp_key(
            m.id,
            &k.server,
            &k.key,
            k.value.as_deref(),
            &ctx.actor.label(),
        )?;
        Ok(Outcome::ok(serde_json::json!({
            "agent": m.name, "server": k.server, "key": k.key,
            "uses": if k.value.is_some() { "its own key" } else { "the system key" },
        })))
    }
}
