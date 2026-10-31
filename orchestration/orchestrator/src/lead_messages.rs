//! Messages to a Lead (ADR-147, ADR-149): the user's message, or an answer to a worker's question routed through the
//! Lead, is stored before it is acknowledged, so a crash loses nothing. While the Lead is busy a message waits in the
//! project's queue, where the user can still change or take it back; the Lead's next chat turn takes every queued
//! message at once, and a turn that does not reach its end puts them back. They are delivered only when the turn
//! reaches its end: a process that dies during the turn finds them taken at the next start and puts them back
//! (reconcile).

use agenium_core::{now_iso, ProjectId};
use agenium_store::core::ledger::{self, LeadMessage};

use crate::app::App;
use crate::{OrchError, Result};

/// Queued messages a chat turn took: put back in the queue unless the turn reached its end ([`Taken::keep`]).
pub struct Taken<'a> {
    app: &'a App,
    pub messages: Vec<LeadMessage>,
    kept: bool,
}

impl Taken<'_> {
    /// The turn had them to its end: they are delivered.
    pub fn keep(mut self) {
        self.kept = true;
        let ids: Vec<i64> = self.messages.iter().map(|m| m.id).collect();
        if let Err(e) = self.app.store.commit(None, "system", |w| {
            ledger::mark_delivered(w.tx, &ids, &now_iso())
        }) {
            // They stay taken: the next start puts them back, and the Lead reads them again rather than never.
            tracing::warn!(error = %e, "messages to the Lead not marked delivered");
        }
    }

    /// The messages as one user message, oldest first.
    pub fn text(&self) -> String {
        self.messages
            .iter()
            .map(|m| m.text.trim())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

impl Drop for Taken<'_> {
    fn drop(&mut self) {
        if self.kept || self.messages.is_empty() {
            return;
        }
        let ids: Vec<i64> = self.messages.iter().map(|m| m.id).collect();
        if let Err(e) = self
            .app
            .store
            .commit(None, "system", |w| ledger::mark_undelivered(w.tx, &ids))
        {
            tracing::warn!(error = %e, "messages to the Lead not put back in the queue");
        }
    }
}

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

    /// The project's messages waiting for its Lead, oldest first.
    pub fn undelivered_lead_messages(&self, project: ProjectId) -> Result<Vec<LeadMessage>> {
        Ok(self.store.read(|c| ledger::undelivered(c, project))?)
    }

    /// A chat turn takes every queued message at once (none can be changed after that).
    pub fn take_lead_messages(&self, project: ProjectId) -> Result<Taken<'_>> {
        let messages = self
            .store
            .commit(None, "system", |w| {
                let ms = ledger::undelivered(w.tx, project)?;
                let ids: Vec<i64> = ms.iter().map(|m| m.id).collect();
                ledger::mark_taken(w.tx, &ids, &now_iso())?;
                Ok::<_, agenium_store::StoreError>(ms)
            })?
            .value;
        Ok(Taken {
            app: self,
            messages,
            kept: false,
        })
    }

    /// Changes a queued message before the Lead reads it (ADR-149).
    pub fn edit_lead_message(&self, project: ProjectId, id: i64, text: &str) -> Result<()> {
        if text.trim().is_empty() {
            return Err(OrchError::Invalid("the message is empty".into()));
        }
        let changed = self
            .store
            .commit(None, "user", |w| {
                ledger::edit_queued(w.tx, project, id, text)
            })?
            .value;
        changed
            .then_some(())
            .ok_or_else(|| OrchError::Invalid("the Lead has read this message already".into()))
    }

    /// Takes a queued message back before the Lead reads it (ADR-149).
    pub fn drop_lead_message(&self, project: ProjectId, id: i64) -> Result<()> {
        let dropped = self
            .store
            .commit(None, "user", |w| ledger::drop_queued(w.tx, project, id))?
            .value;
        dropped
            .then_some(())
            .ok_or_else(|| OrchError::Invalid("the Lead has read this message already".into()))
    }
}
