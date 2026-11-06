//! An agent whose provider could not be reached after the retries of a run (an overloaded service, a network that
//! went away) is tried again by itself (ADR-158): two minutes later, then twice as long each time, 30 minutes at most,
//! six tries in all; after them it waits for the user. A CLI that cannot run or a provider without a key waits for the
//! user at once (`set_aside`): trying again would not help. Seen in the research run: OpenCode answered "overloaded"
//! for a minute and the Lead's review stopped the whole project until the user resumed it.

use std::time::{Duration, SystemTime};

use super::*;

const FIRST_WAIT: Duration = Duration::from_secs(2 * 60);
const MAX_WAIT: Duration = Duration::from_secs(30 * 60);
/// Tries before the agent waits for the user.
pub(super) const MAX_TRIES: u32 = 6;

#[derive(Debug, Clone, Copy)]
pub(super) struct OfflineWait {
    at: SystemTime,
    tries: u32,
}

/// The wait before try `n` (1-based).
fn wait(n: u32) -> Duration {
    FIRST_WAIT
        .saturating_mul(1 << n.saturating_sub(1).min(8))
        .min(MAX_WAIT)
}

impl Runtime {
    /// A run of `agent` ended Offline at `now`: when it is tried again, `None` once its tries are spent.
    pub fn offline_later(&self, agent: AgentId, now: SystemTime) -> Option<Duration> {
        let mut waits = self.offline_waits.lock().unwrap_or_else(|p| p.into_inner());
        let w = waits
            .entry(agent)
            .or_insert(OfflineWait { at: now, tries: 0 });
        if w.tries >= MAX_TRIES {
            // Spent: no round brings it back any more, only the user.
            w.tries = MAX_TRIES + 1;
            return None;
        }
        w.tries += 1;
        let d = wait(w.tries);
        w.at = now + d;
        Some(d)
    }

    /// A run of `agent` ended otherwise: its provider answers, its next offline spell starts the waits afresh.
    pub(super) fn offline_over(&self, agent: AgentId) {
        self.offline_waits
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&agent);
    }

    /// Sets back to work the agents whose wait is over at `now`; returns them. Run before each round.
    pub fn offline_back_round(&self, now: SystemTime) -> Result<Vec<AgentId>> {
        let agents = self.app.agents()?;
        let due: Vec<AgentId> = {
            let waits = self.offline_waits.lock().unwrap_or_else(|p| p.into_inner());
            waits
                .iter()
                .filter(|(id, w)| {
                    now >= w.at
                        && w.tries <= MAX_TRIES
                        && agents
                            .iter()
                            .any(|a| a.id == **id && a.status == AgentStatus::Offline)
                })
                .map(|(id, _)| *id)
                .collect()
        };
        for id in &due {
            self.app.set_agent_status(*id, AgentStatus::Idle)?;
        }
        Ok(due)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_waits_double_up_to_half_an_hour() {
        let m = |n| wait(n).as_secs() / 60;
        assert_eq!([m(1), m(2), m(3), m(4), m(5), m(6)], [2, 4, 8, 16, 30, 30]);
    }
}
