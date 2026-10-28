//! The user closes a project once its work is done (ADR-130): nothing runs on it any more and its team is set free,
//! members and Lead together, to take another project. Only the user closes a project, and only the user reopens
//! one: no agent tool does either. The mark is a project setting (`closed_at`; the stored status stays Completed);
//! the team that worked on it is remembered (`closed_team`), so reopening can offer it again: it knows the project
//! (its memory of it), a new team starts fresh. When it is busy on another project, the reopened project may wait
//! for it (`waits_for_team`) and gets it as soon as it is set free (`hand_over`).

use agenium_core::events::DomainEvent;
use agenium_core::{AgentStatus, ProjectId, TeamId};
use agenium_store::core::projects::{self, ProjectStatus};
use agenium_store::core::{agents, runs};

use crate::app::App;
use crate::views::RunState;
use crate::{OrchError, Result};

/// The project setting that marks a closed project: when the user closed it.
pub const CLOSED_AT: &str = "closed_at";
/// The project setting naming the team that worked on a closed project.
pub const CLOSED_TEAM: &str = "closed_team";
/// The project setting naming the team a reopened project waits for (busy on another project when it was reopened).
pub const WAITS_FOR: &str = "waits_for_team";

/// Who takes a reopened project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taker {
    /// A free team, now.
    Team(TeamId),
    /// A team busy on another project: it takes this one as soon as it is free.
    WaitFor(TeamId),
    /// Nobody yet (the user gives it a team in Teams).
    Nobody,
}

/// Whether the project settings `s` mark a closed project.
pub fn closed(s: &serde_json::Value) -> bool {
    s.get(CLOSED_AT).is_some_and(|v| !v.is_null())
}

impl App {
    /// Closes a done project and sets its team free; returns the team set free and the project waiting for it that
    /// took it at once, if any. Refused while work is open (it is not done) or one of its agents still works.
    pub fn close_project(&self, project: ProjectId) -> Result<(Option<TeamId>, Option<ProjectId>)> {
        match self.run_state(project)? {
            RunState::Done => {}
            RunState::Closed => return Err(OrchError::Conflict("the project is closed already".into())),
            s => {
                return Err(OrchError::Conflict(format!(
                    "the project is not done ({s:?}): its plan must be done and no ticket open before it closes"
                )))
            }
        }
        let team = self.team_of_project(project)?;
        // Any run of the team still going (a ticket's, or the Lead answering in the chat, which marks no one
        // Working): it could still change the project, so it ends first.
        let members: Vec<&agents::Agent> = team.iter().flat_map(|(_, m)| m).collect();
        let live = self.store().read(runs::unfinished)?;
        if let Some(a) = members
            .iter()
            .find(|m| m.status == AgentStatus::Working || live.iter().any(|r| r.agent == m.id))
        {
            return Err(OrchError::Conflict(format!(
                "{} is at work on the project; wait until its run ends",
                a.name
            )));
        }
        let now = chrono::Utc::now().to_rfc3339();
        self.store().commit(None, "user", |w| -> Result<()> {
            let mut s = projects::get(w.tx, project)?.settings;
            if !s.is_object() {
                s = serde_json::json!({});
            }
            s[CLOSED_AT] = serde_json::json!(now);
            s[CLOSED_TEAM] = serde_json::json!(team.as_ref().map(|(t, _)| t.id));
            projects::set_settings(w.tx, project, &s)?;
            projects::set_status(w.tx, project, ProjectStatus::Completed)?;
            w.emit(DomainEvent::ProjectStatusChanged {
                project,
                status: "Closed".into(),
            })?;
            if let Some((t, _)) = &team {
                agents::set_team_project(w.tx, t.id, None)?;
                w.emit(DomainEvent::TeamProjectChanged {
                    team: t.id,
                    project: None,
                })?;
            }
            Ok(())
        })?;
        let freed = team.map(|(t, _)| t.id);
        let taken = match freed {
            Some(t) => self.hand_over(t)?,
            None => None,
        };
        Ok((freed, taken))
    }

    /// Reopens a closed project (only the user): it goes on from where it was closed, taken by a free team now, by a
    /// busy one once it is free (the team that worked on it knows it best; another starts fresh), or by nobody yet.
    pub fn reopen_project(&self, project: ProjectId, taker: Taker) -> Result<()> {
        if !closed(&self.project(project)?.settings) {
            return Err(OrchError::Conflict("the project is not closed".into()));
        }
        // Waiting for a team that is free already is taking it now.
        let taker = match taker {
            Taker::WaitFor(t) if self.team(t)?.0.project.is_none() => Taker::Team(t),
            other => other,
        };
        let team = match taker {
            Taker::Team(t) => Some(t),
            _ => None,
        };
        if let Some(t) = team {
            let (tt, _) = self.team(t)?;
            if tt.project.is_some() {
                return Err(OrchError::Conflict(format!(
                    "{} works on another project: close that one or pick a free team",
                    tt.name
                )));
            }
        }
        self.store().commit(None, "user", |w| -> Result<()> {
            let mut s = projects::get(w.tx, project)?.settings;
            if let Some(o) = s.as_object_mut() {
                o.remove(CLOSED_AT);
                o.remove(WAITS_FOR);
            }
            if let Taker::WaitFor(t) = taker {
                s[WAITS_FOR] = serde_json::json!(t);
            }
            projects::set_settings(w.tx, project, &s)?;
            projects::set_status(w.tx, project, ProjectStatus::Active)?;
            w.emit(DomainEvent::ProjectStatusChanged {
                project,
                status: "Active".into(),
            })?;
            if let Some(t) = team {
                agents::set_team_project(w.tx, t, Some(project))?;
                w.emit(DomainEvent::TeamProjectChanged {
                    team: t,
                    project: Some(project),
                })?;
            }
            Ok(())
        })?;
        Ok(())
    }

    /// The team that worked on a closed project, while it still exists.
    pub fn previous_team(&self, project: ProjectId) -> Result<Option<TeamId>> {
        let s = self.project(project)?.settings;
        let id = s
            .get(CLOSED_TEAM)
            .and_then(|v| v.as_str())
            .and_then(|v| v.parse().ok());
        Ok(id.filter(|t| self.team(*t).is_ok()))
    }

    /// The team a reopened project waits for, while it still exists.
    pub fn waits_for(&self, project: ProjectId) -> Result<Option<TeamId>> {
        let s = self.project(project)?.settings;
        let id = s
            .get(WAITS_FOR)
            .and_then(|v| v.as_str())
            .and_then(|v| v.parse().ok());
        Ok(id.filter(|t| self.team(*t).is_ok()))
    }

    /// A team set free goes to the project that waits for it, if any (the first one); returns that project.
    pub fn hand_over(&self, team: TeamId) -> Result<Option<ProjectId>> {
        if self.team(team)?.0.project.is_some() {
            return Ok(None);
        }
        let mine = serde_json::json!(team);
        let Some(p) = self
            .projects()?
            .into_iter()
            .find(|p| p.settings.get(WAITS_FOR) == Some(&mine) && !closed(&p.settings))
        else {
            return Ok(None);
        };
        let has_team = self.team_of_project(p.id)?.is_some();
        self.store().commit(None, "user", |w| -> Result<()> {
            let mut s = projects::get(w.tx, p.id)?.settings;
            if let Some(o) = s.as_object_mut() {
                o.remove(WAITS_FOR);
            }
            projects::set_settings(w.tx, p.id, &s)?;
            // A project given another team meanwhile waits no more.
            if !has_team {
                agents::set_team_project(w.tx, team, Some(p.id))?;
                w.emit(DomainEvent::TeamProjectChanged {
                    team,
                    project: Some(p.id),
                })?;
            }
            Ok(())
        })?;
        Ok((!has_team).then_some(p.id))
    }
}
