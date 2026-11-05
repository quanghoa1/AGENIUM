//! Tickets that stop moving: the stall nudge, reviews that keep stopping, and when a ticket may start by itself.

use super::*;

/// Prefixes of the System comments of the stall-nudge.
pub(super) const STALL_NOTE: &str = "Stall nudge";

pub(super) const STALL_ESCALATED: &str = "Stall: escalated to the user.";

/// Prefix of the System comment that hands a ticket whose reviews keep stopping at a checkpoint to the user.
pub(super) const REVIEW_STALLED: &str = "Review stall:";

impl Runtime {
    pub(super) fn ready_to_autostart(&self, ticket: TicketId) -> Result<bool> {
        let rs = self.app.store().read(|c| runs::for_ticket(c, ticket))?;
        let tail: Vec<&runs::Run> = rs.iter().rev().take(MAX_FAILED_RUNS).collect();
        Ok(!(tail.len() == MAX_FAILED_RUNS
            && tail
                .iter()
                .all(|r| r.outcome.as_deref().is_some_and(failed))))
    }

    /// Stall-nudge (decision 28) for a ticket whose runs keep stopping: once `stall_minutes` passed since its last
    /// run, it gets another run with a System nudge, up to `stall_nudges` times since its last good run; then it
    /// goes to the user (once). Returns whether to start a run now (the nudge comment is posted).
    pub(super) fn stall_nudge(&self, t: &Ticket) -> Result<bool> {
        let rt = self.app.config().runtime;
        let rs = self.app.store().read(|c| runs::for_ticket(c, t.id))?;
        let Some(last) = rs.last() else {
            return Ok(false);
        };
        let Some(ended) = last.ended_at.as_deref() else {
            return Ok(false);
        };
        let since_good = rs
            .iter()
            .rev()
            .find(|r| r.outcome.as_deref().is_some_and(|o| !failed(o)))
            .and_then(|r| r.ended_at.clone())
            .unwrap_or_default();
        let idle = chrono::DateTime::parse_from_rfc3339(ended)
            .map(|e| chrono::Utc::now().signed_duration_since(e))
            .unwrap_or_default();
        if idle < chrono::Duration::minutes(i64::from(rt.stall_minutes)) {
            return Ok(false);
        }
        let cs = self.app.store().read(|c| comments::for_ticket(c, t.id))?;
        let system = |prefix: &str| {
            cs.iter()
                .filter(|c| c.created_at.as_str() > since_good.as_str())
                .filter(
                    |c| matches!(&c.body, CommentBody::System { text } if text.starts_with(prefix)),
                )
                .count() as u32
        };
        let given = system(STALL_NOTE);
        let outcome = last.outcome.clone().unwrap_or_default();
        if given >= rt.stall_nudges {
            if system(STALL_ESCALATED) == 0 {
                self.app.store().commit(None, "system", |w| -> Result<()> {
                    let tk = tickets::get(w.tx, t.id)?;
                    crate::engine::system_comment(w, &tk, format!("{STALL_ESCALATED} runs on {} keep stopping ({outcome}) after {given} nudge(s); it waits for you (look at the run log, change the model or the ticket).", t.key))?;
                    w.emit(DomainEvent::NeedsUser { ticket: Some(t.id), reason: format!("{}: runs keep stopping ({outcome}); look at the run log", t.key) })?;
                    Ok(())
                })?;
            }
            return Ok(false);
        }
        self.app.store().commit(None, "system", |w| -> Result<()> {
            let tk = tickets::get(w.tx, t.id)?;
            crate::engine::system_comment(w, &tk, format!("{STALL_NOTE} {}/{}: your last runs on {} stopped ({outcome}). Resume the work from where it is, in small verified steps.", given + 1, rt.stall_nudges, t.key))?;
            Ok(())
        })?;
        Ok(true)
    }

    /// Whether the Lead's reviews of `t` keep ending at a checkpoint (max_steps, context) without a decision. A review
    /// starts fresh each time, so it would run again every round: after MAX_FAILED_RUNS in a row since the user's last
    /// message to the Lead (a hint gives the Lead another try) it waits for the user, who is told once.
    pub(super) fn review_stalled(&self, t: &Ticket) -> Result<bool> {
        let rs = self.app.store().read(|c| runs::for_ticket(c, t.id))?;
        let cs = self.app.store().read(|c| comments::for_ticket(c, t.id))?;
        let hint = self
            .app
            .store()
            .read(|c| agenium_store::core::ledger::last_message_at(c, t.project))?
            .unwrap_or_default();
        let hint = hint.as_str();
        let undecided: Vec<&runs::Run> = rs
            .iter()
            .rev()
            .take_while(|r| {
                r.started_at.as_str() > hint
                    && r.outcome
                        .as_deref()
                        .is_some_and(|o| o.starts_with("Checkpoint"))
                    && self.is_review_run(r)
            })
            .collect();
        if undecided.len() < MAX_FAILED_RUNS {
            return Ok(false);
        }
        // The earliest review of the row: the user was told once since it.
        let since = &undecided[undecided.len() - 1].started_at;
        let told = cs.iter().any(|c| {
            c.created_at > *since
                && matches!(&c.body, CommentBody::System { text } if text.starts_with(REVIEW_STALLED))
        });
        if !told {
            let outcome = undecided[0].outcome.clone().unwrap_or_default();
            self.app.store().commit(None, "system", |w| -> Result<()> {
                let tk = tickets::get(w.tx, t.id)?;
                crate::engine::system_comment(w, &tk, format!("{REVIEW_STALLED} {} reviews of {} in a row stopped at a checkpoint ({outcome}) without a decision; it waits for you (tell the Lead a hint to give it another try, or give the Lead more steps or another model).", undecided.len(), t.key))?;
                w.emit(DomainEvent::NeedsUser { ticket: Some(t.id), reason: format!("{}: the Lead's reviews keep stopping at a checkpoint ({outcome}); look at the run log", t.key) })?;
                Ok(())
            })?;
        }
        Ok(true)
    }
}
