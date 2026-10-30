//! Messages to a Lead that survive a crash (ADR-147): the user's message, or an answer to a worker's question routed
//! through the Lead, is stored before it is acknowledged; it counts as delivered once a run of the Lead had it in its
//! conversation. Undelivered messages go to the Lead again after a restart: a message is never lost between the
//! acknowledgment and the Lead's turn (it may at worst be read twice).

use agenium_core::{now_iso, ProjectId};
use agenium_store::core::ledger::{self, LeadMessage};

use crate::app::App;
use crate::Result;

impl App {
    /// Stores a message to the Lead of `project` (before it is acknowledged); returns its id.
    pub fn post_lead_message(&self, project: ProjectId, text: &str) -> Result<i64> {
        Ok(self
            .store
            .commit(None, "user", |w| {
                ledger::post_lead_message(w.tx, project, text, &now_iso())
            })?
            .value)
    }

    /// The project's messages to its Lead that no run of the Lead has had yet, oldest first.
    pub fn undelivered_lead_messages(&self, project: ProjectId) -> Result<Vec<LeadMessage>> {
        Ok(self.store.read(|c| ledger::undelivered(c, project))?)
    }

    /// Marks delivered the project's undelivered messages that a run's conversation carries (`seen`: its user
    /// messages, a steered message among them).
    pub fn lead_messages_seen(&self, project: ProjectId, seen: &[String]) -> Result<usize> {
        let ids: Vec<i64> = self
            .undelivered_lead_messages(project)?
            .into_iter()
            .filter(|m| {
                let t = m.text.trim();
                !t.is_empty() && seen.iter().any(|s| s.contains(t))
            })
            .map(|m| m.id)
            .collect();
        if !ids.is_empty() {
            self.store.commit(None, "system", |w| {
                ledger::mark_delivered(w.tx, &ids, &now_iso())
            })?;
        }
        Ok(ids.len())
    }
}
