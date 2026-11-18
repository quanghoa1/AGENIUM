//! The Lead's inbox in a round: questions, permissions and reports it has to act on.

use super::*;

impl Runtime {
    /// Runs the Lead on its inbox when the Lead is Idle and the same inbox did not fail twice in a row.
    pub(super) async fn inbox_round(&self, project: ProjectId) -> Result<Option<RunSummary>> {
        let items = self.app.lead_inbox(project)?;
        if items.is_empty() {
            return Ok(None);
        }
        let Some((team, _)) = self.app.team_of_project(project)? else {
            return Ok(None);
        };
        let Some(lead) = team.lead else {
            return Ok(None);
        };
        if self.app.agent(lead)?.status != AgentStatus::Idle {
            return Ok(None);
        }
        // Not while the Lead has a run going (a chat turn marks no one Working): an inbox run beside it planned the
        // same work at once, and each rewrote what the other had made (a Solo's plan revised three times, its tickets
        // cancelled; 2026-11-05). Runs a crash left open are closed at start (`reconcile`).
        if self.lead_busy(lead)
            || self
                .app
                .store()
                .read(agenium_store::core::runs::unfinished)?
                .iter()
                .any(|r| r.agent == lead)
        {
            return Ok(None);
        }
        let signature = inbox_signature(&items);
        let failures = |m: &Mutex<std::collections::HashMap<ProjectId, (String, usize)>>| {
            m.lock()
                .unwrap_or_else(|p| p.into_inner())
                .get(&project)
                .filter(|(s, _)| *s == signature)
                .map_or(0, |(_, n)| *n)
        };
        if failures(&self.inbox_failures) >= MAX_FAILED_RUNS {
            return Ok(None);
        }
        // A run that fails before its loop counts like one that did not empty the inbox (else it is retried every
        // round); one the user stopped does not count.
        let (s, outcome) = match self.inbox(project).await {
            Ok(Some(s)) => {
                let o = s.outcome.clone();
                (Some(s), o)
            }
            Ok(None) => return Ok(None),
            Err(e) => {
                tracing::warn!(error = %e, "Lead inbox run failed");
                (None, format!("Error:{e}"))
            }
        };
        let mut m = self
            .inbox_failures
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        // The Lead answered without acting (a user who said to wait, items it chose to leave): its words go to the user
        // once, and the same items are not run again until the inbox changes. Before, it ran twice and then told the
        // user the Lead "could not handle its inbox" (2026-11-05).
        let held = s
            .as_ref()
            .filter(|s| s.outcome == "Stopped:no_tool_call" && !s.text.trim().is_empty())
            .map(|s| s.text.trim().chars().take(600).collect::<String>());
        if let Some(text) = held {
            m.insert(project, (signature, MAX_FAILED_RUNS));
            drop(m);
            self.app.store().commit(None, "system", |w| {
                w.emit(DomainEvent::NeedsUser {
                    ticket: None,
                    reason: format!(
                        "{}: the Lead left {} inbox item(s) as they are: {text}",
                        self.app
                            .project(project)
                            .map(|p| p.name)
                            .unwrap_or_default(),
                        items.len()
                    ),
                })
                .map_err(OrchError::from)
            })?;
            return Ok(s);
        }
        if outcome == "Completed" {
            m.remove(&project);
        } else if outcome != "Cancelled" {
            let n = m
                .get(&project)
                .filter(|(sig, _)| *sig == signature)
                .map_or(0, |(_, n)| *n)
                + 1;
            m.insert(project, (signature, n));
            if n >= MAX_FAILED_RUNS {
                drop(m);
                self.app.store().commit(None, "system", |w| {
                    w.emit(DomainEvent::NeedsUser {
                        ticket: None,
                        reason: format!(
                            "{}: the Lead could not handle its inbox ({} item(s)) in {n} runs ({}); look at the run log",
                            self.app.project(project).map(|p| p.name).unwrap_or_default(),
                            items.len(),
                            outcome
                        ),
                    })
                    .map_err(OrchError::from)
                })?;
            }
        }
        Ok(s)
    }
}

/// What the Lead's inbox holds, to tell whether it changed.
pub(super) fn inbox_signature(items: &[crate::inbox::InboxItem]) -> String {
    items
        .iter()
        .map(|i| format!("{:?}:{:?}:{:?}", i.kind, i.ticket, i.comment))
        .collect::<Vec<_>>()
        .join("|")
}
