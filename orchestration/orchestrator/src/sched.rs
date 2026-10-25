//! Scheduling (decision 36 / architecture "Scheduler & vận hành"): workers never pull work; the Lead assigns and
//! the scheduler starts a run when the ticket is ready (dependencies satisfied) and its assignee is Idle.
//! Priority first, then age, and waiting raises the priority (aging, [`effective_priority`]). Also the run
//! lifecycle bookkeeping used by agent runtimes.

use agenium_core::*;
use agenium_store::core::{agents, runs, tickets};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::engine::{deps_satisfied, transition};
use crate::{OrchError, Result};

/// A ticket ready for a run and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Ready {
    pub ticket: Ticket,
    pub agent: AgentId,
    /// `true` when the ticket is already InProgress (answer arrived, crash, max_steps checkpoint): the run
    /// continues the previous conversation.
    pub continuation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunStart {
    pub run: RunId,
    pub ticket: TicketId,
    pub agent: AgentId,
    pub continuation: bool,
}

/// The priority a ticket is scheduled with: one level higher (lower number) for every `aging_hours` since it was
/// created, never above P0, so low-priority work is not starved by a stream of new urgent tickets.
pub fn effective_priority(
    priority: u8,
    created_at: &str,
    now: chrono::DateTime<chrono::Utc>,
    aging_hours: u32,
) -> u8 {
    if aging_hours == 0 {
        return priority;
    }
    let Ok(at) = chrono::DateTime::parse_from_rfc3339(created_at) else {
        return priority;
    };
    let waited = now.signed_duration_since(at).num_hours().max(0) as u64;
    let steps = waited / u64::from(aging_hours);
    priority.saturating_sub(steps.min(u64::from(u8::MAX)) as u8)
}

impl App {
    /// Tickets of `project` that can start a run now, best first. At most one per agent (one active ticket per
    /// worker, decision 28).
    pub fn ready_tickets(&self, project: ProjectId) -> Result<Vec<Ready>> {
        let ts = self.tickets(project)?;
        let mut out: Vec<Ready> = Vec::new();
        let live: std::collections::HashSet<TicketId> = self
            .store
            .read(runs::unfinished)?
            .into_iter()
            .filter_map(|r| r.ticket)
            .collect();
        let mut cands: Vec<&Ticket> = ts
            .iter()
            .filter(|t| {
                matches!(
                    t.status,
                    TicketStatus::Assigned | TicketStatus::Reopened | TicketStatus::InProgress
                )
            })
            .filter(|t| t.assignee.is_some() && !live.contains(&t.id))
            .collect();
        // Effective priority (P0 first, raised by waiting), then oldest first.
        let (now, aging) = (chrono::Utc::now(), self.config().runtime.aging_hours);
        let eff = |t: &Ticket| effective_priority(t.priority, &t.created_at, now, aging);
        cands.sort_by(|a, b| eff(a).cmp(&eff(b)).then(a.created_at.cmp(&b.created_at)));
        for t in cands {
            let agent = t.assignee.unwrap_or_else(AgentId::new);
            if out.iter().any(|r| r.agent == agent) {
                continue;
            }
            let a = self.agent(agent)?;
            if a.status != AgentStatus::Idle {
                continue;
            }
            let continuation = t.status == TicketStatus::InProgress;
            if !continuation
                && !self.store.read(|c| {
                    deps_satisfied(c, t)
                        .map_err(|e| agenium_store::StoreError::Invalid(e.to_string()))
                })?
            {
                continue;
            }
            // An agent already working on another InProgress ticket keeps it (one active ticket per worker).
            let busy_elsewhere = ts.iter().any(|o| {
                o.id != t.id
                    && o.assignee == Some(agent)
                    && matches!(o.status, TicketStatus::InProgress | TicketStatus::Pending)
            });
            if busy_elsewhere && !continuation {
                continue;
            }
            out.push(Ready {
                ticket: t.clone(),
                agent,
                continuation,
            });
        }
        Ok(out)
    }

    /// Starts a run on a ready ticket: Assigned/Reopened -> InProgress (System), agent -> Working, run row.
    pub fn start_run(
        &self,
        project: ProjectId,
        key: &TicketKey,
        provider: &str,
        model: &str,
    ) -> Result<RunStart> {
        let run = RunId::new();
        let out = self.store.commit(None, "system", |w| -> Result<RunStart> {
            let mut t = tickets::get_by_key(w.tx, project, key)?;
            let agent_id = t
                .assignee
                .ok_or_else(|| OrchError::Invalid(format!("{key} has no assignee")))?;
            let agent = agents::get_agent(w.tx, agent_id)?;
            if agent.status != AgentStatus::Idle {
                return Err(OrchError::Invalid(format!(
                    "{} is {}",
                    agent.name, agent.status
                )));
            }
            if runs::unfinished(w.tx)?
                .iter()
                .any(|r| r.ticket == Some(t.id))
            {
                return Err(OrchError::Conflict(format!("{key} already has a live run")));
            }
            let continuation = t.status == TicketStatus::InProgress;
            if !continuation {
                let g = Guards {
                    deps_satisfied: deps_satisfied(w.tx, &t)?,
                    assignee_idle: true,
                    ..Default::default()
                };
                transition(w, &mut t, &Actor::System, TicketStatus::InProgress, &g)?;
            }
            let from = agents::set_agent_status(w.tx, agent_id, AgentStatus::Working)?;
            w.emit(DomainEvent::AgentStatusChanged {
                agent: agent_id,
                from,
                to: AgentStatus::Working,
            })?;
            runs::insert(
                w.tx,
                &runs::Run {
                    id: run,
                    ticket: Some(t.id),
                    agent: agent_id,
                    provider: provider.to_owned(),
                    model: model.to_owned(),
                    started_at: now_iso(),
                    ended_at: None,
                    outcome: None,
                    steps: 0,
                    usage: UsageSummary::default(),
                    heartbeat: Some(now_iso()),
                },
            )?;
            w.emit(DomainEvent::RunStarted {
                run,
                agent: agent_id,
                ticket: Some(t.id),
            })?;
            Ok(RunStart {
                run,
                ticket: t.id,
                agent: agent_id,
                continuation,
            })
        })?;
        Ok(out.value)
    }

    /// Records progress of a live run (heartbeat for the watchdog, display-only usage).
    pub fn run_progress(&self, run: RunId, steps: u32, usage: &UsageSummary) -> Result<()> {
        self.store.commit(None, "system", |w| -> Result<()> {
            runs::update_progress(w.tx, run, steps, usage, &now_iso())?;
            Ok(())
        })?;
        Ok(())
    }

    /// Ends a run: outcome recorded, usage added to the ticket (display only), agent back to Idle unless it
    /// is Waiting on a blocking item or its quota/connectivity state was set.
    pub fn finish_run(
        &self,
        run: RunId,
        outcome: &str,
        steps: u32,
        usage: &UsageSummary,
    ) -> Result<()> {
        let ticket = self
            .store
            .commit(None, "system", |w| -> Result<Option<TicketId>> {
                let r = runs::get(w.tx, run)?;
                if r.ended_at.is_some() {
                    return Ok(None);
                }
                runs::update_progress(w.tx, run, steps, usage, &now_iso())?;
                runs::finish(w.tx, run, &now_iso(), outcome)?;
                if let Some(tid) = r.ticket {
                    let mut t = tickets::get(w.tx, tid)?;
                    t.usage.add(usage);
                    tickets::update(w.tx, &t)?;
                }
                let a = agents::get_agent(w.tx, r.agent)?;
                if a.status == AgentStatus::Working {
                    agents::set_agent_status(w.tx, r.agent, AgentStatus::Idle)?;
                    w.emit(DomainEvent::AgentStatusChanged {
                        agent: r.agent,
                        from: AgentStatus::Working,
                        to: AgentStatus::Idle,
                    })?;
                }
                w.emit(DomainEvent::RunEnded {
                    run,
                    agent: r.agent,
                    outcome: outcome.to_owned(),
                })?;
                Ok(r.ticket)
            })?;
        // Decision 19: what the run left in the worktree, to tell later outside changes from the agents' work.
        if let Some(t) = ticket.value {
            let project = self.store.read(|c| tickets::get(c, t))?.project;
            self.record_worktree_state(project, run)?;
        }
        Ok(())
    }

    /// Appends to the structured run log (diagnostics/replay), redacting secrets.
    pub fn run_log(&self, run: RunId, kind: &str, data: serde_json::Value) -> Result<()> {
        let text = self.redactor.redact(&data.to_string());
        let data: serde_json::Value =
            serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text));
        self.store.commit(None, "system", |w| -> Result<()> {
            runs::log(w.tx, run, kind, &data)?;
            Ok(())
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::effective_priority;

    #[test]
    fn waiting_raises_the_priority_up_to_p0() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-25T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let fresh = "2026-10-25T11:00:00Z";
        let day = "2026-10-24T11:00:00Z";
        let week = "2026-10-18T12:00:00Z";
        assert_eq!(effective_priority(2, fresh, now, 24), 2);
        assert_eq!(
            effective_priority(2, day, now, 24),
            1,
            "a day of waiting: one level"
        );
        assert_eq!(effective_priority(3, week, now, 24), 0, "never above P0");
        assert_eq!(effective_priority(2, week, now, 0), 2, "aging off");
        // A P2 that waited a day ranks with a fresh P1 and, being older, goes first.
        assert!(effective_priority(2, day, now, 24) <= effective_priority(1, fresh, now, 24));
    }
}
