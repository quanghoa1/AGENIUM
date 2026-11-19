//! A worker's run Agenium stopped because the worker was stuck (the same call over and over, or its calls failing in a
//! row): asked on the worker's behalf as a blocking Blocker question, so its Lead (the user, for a Solo agent)
//! sees it and says how to go on, and the worker's next run reads the answer. 2026-11-19: such a ticket went quiet,
//! telling no one, until the stall nudges ran out.

use super::*;

impl App {
    /// Asks how to go on for the stuck run on `key`: `why` is what stopped it, `context` its last call and result.
    /// Nothing is asked when the ticket is no longer this worker's to work on.
    pub fn ask_for_stuck_run(
        &self,
        project: ProjectId,
        key: &TicketKey,
        agent: AgentId,
        why: &str,
        context: &str,
    ) -> Result<()> {
        let question: String = format!(
            "Agenium stopped my run: {why}. How should I go on: another approach, something fixed in the environment, \
             or a change to the ticket?"
        )
        .chars()
        .take(500)
        .collect();
        self.store.commit(None, "system", |w| -> Result<()> {
            let mut t = ticket_by_key(w, project, key)?;
            if t.assignee != Some(agent)
                || !matches!(t.status, TicketStatus::InProgress | TicketStatus::Pending)
            {
                return Ok(());
            }
            let q = Question {
                ticket_id: key.clone(),
                category: QuestionCategory::Blocker,
                question,
                context: context.chars().take(1500).collect(),
                options: vec![],
                recommendation: None,
                blocking: true,
                urgency: Urgency::High,
            };
            add_comment(w, &t, Author::Agent(agent), None, CommentBody::Question(q))?;
            if t.status == TicketStatus::InProgress {
                transition(
                    w,
                    &mut t,
                    &Actor::Worker(agent),
                    TicketStatus::Pending,
                    &Guards {
                        open_blocking: true,
                        ..Default::default()
                    },
                )?;
            }
            set_agent_status(w, agent, AgentStatus::Waiting)?;
            // A Solo agent is its own Lead: the user is the one to say how to go on.
            if solo_agent(w.tx, agent)? {
                w.emit_as(
                    "system",
                    DomainEvent::NeedsUser {
                        ticket: Some(t.id),
                        reason: format!("{} is stuck and asks how to go on", t.key),
                    },
                )?;
            }
            Ok(())
        })?;
        Ok(())
    }
}
