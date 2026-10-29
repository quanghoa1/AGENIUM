//! An agent set aside for its quota goes back to work once the quota is back (ADR-135), with the same model: never
//! another (ADR-132). When its vendor said when the used-up window resets (the subscription usage Agenium reads: Claude
//! Code, Codex, OpenCode Go), it goes back then; otherwise it is tried again after a while, 30 minutes and then twice as
//! long each time, 4 hours at most. A try that finds the quota still used up sets it aside again until the next one.
//! Its tickets go on from where they stopped; the user can still resume it at once or give it another model.

use std::time::{Duration, SystemTime};

use agenium_store::core::agents::Agent;

use super::*;

/// The first wait when the vendor did not say when its window resets.
const FIRST_WAIT: Duration = Duration::from_secs(30 * 60);
/// The longest wait between two tries.
const MAX_WAIT: Duration = Duration::from_secs(4 * 3600);

/// When an agent set aside for its quota is tried again, and how many tries it had.
#[derive(Debug, Clone, Copy)]
pub(super) struct QuotaWait {
    at: SystemTime,
    tries: u32,
}

impl Runtime {
    /// Sets back to work the agents whose quota should be back at `now`; returns them. Run before each round.
    pub fn quota_back_round(&self, now: SystemTime) -> Result<Vec<AgentId>> {
        let agents = self.app.agents()?;
        let mut back = Vec::new();
        let mut waits = self.quota_waits.lock().unwrap_or_else(|p| p.into_inner());
        // One that went back to work (or was resumed by the user) starts afresh the next time.
        waits.retain(|id, _| {
            agents
                .iter()
                .any(|a| a.id == *id && a.status == AgentStatus::QuotaExhausted)
        });
        for a in agents
            .iter()
            .filter(|a| a.status == AgentStatus::QuotaExhausted)
        {
            let reset = self.window_reset(a, now);
            let w = waits.entry(a.id).or_insert_with(|| QuotaWait {
                at: reset.unwrap_or(now + FIRST_WAIT),
                tries: 0,
            });
            if now < w.at {
                continue;
            }
            w.tries += 1;
            let wait = FIRST_WAIT.saturating_mul(1 << w.tries.min(8)).min(MAX_WAIT);
            w.at = now + wait;
            let why = match reset {
                Some(_) => "its vendor's window has reset".to_owned(),
                None if w.tries == 1 => format!("{} minutes passed", FIRST_WAIT.as_secs() / 60),
                None => "tried again".to_owned(),
            };
            back.push((a.clone(), why));
        }
        drop(waits);
        for (a, why) in &back {
            self.app.set_agent_status(a.id, AgentStatus::Idle)?;
            self.note_quota_back(a, why)?;
        }
        Ok(back.into_iter().map(|(a, _)| a.id).collect())
    }

    /// When the window that set `a` aside resets, as its vendor last reported (a window used up, its reset to come).
    fn window_reset(&self, a: &Agent, now: SystemTime) -> Option<SystemTime> {
        let vendor = match a.backend {
            BackendKind::Cli => a.model.provider.clone(),
            BackendKind::Api => format!("opencode-go:{}", a.model.provider),
        };
        let reading = self.app.vendor_reading(&vendor)?;
        reading
            .windows
            .iter()
            .filter(|w| w.used_percent >= 99.0)
            .filter_map(|w| w.resets_at.as_deref())
            .filter_map(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| SystemTime::from(t) + Duration::from_secs(60))
            .filter(|t| *t > now)
            .max()
    }

    /// Says on the agent's open tickets that it goes on.
    fn note_quota_back(&self, a: &Agent, why: &str) -> Result<()> {
        let text = format!(
            "{}'s quota on {} should be back ({why}): it goes on from where it stopped, with the same model.",
            a.name, a.model.model
        );
        for p in self.app.projects()? {
            for t in self.app.tickets(p.id)? {
                let open = matches!(
                    t.status,
                    TicketStatus::Assigned | TicketStatus::InProgress | TicketStatus::Reopened
                );
                if open && t.assignee == Some(a.id) {
                    self.app.store().commit(None, "system", |w| -> Result<()> {
                        crate::engine::system_comment(w, &t, text.clone())?;
                        Ok(())
                    })?;
                }
            }
        }
        Ok(())
    }
}
