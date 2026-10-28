//! Questions on tickets: a worker asks, the Lead (or the user, through the Lead) answers. Split out of `engine.rs`.

use super::*;

impl App {
    pub(super) fn ask_question(&self, ctx: &CallCtx, q: Question) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            let mut t = ticket_by_key(w, pid, &q.ticket_id)?;
            let agent = require_assignee(&t, &ctx.actor)?;
            if t.status != TicketStatus::InProgress && t.status != TicketStatus::Pending {
                return Err(invalid(format!("{} is {}; questions are asked while working on it", t.key, t.status)));
            }
            let blocking = q.blocking;
            let c = add_comment(w, &t, Author::Agent(agent), None, CommentBody::Question(q))?;
            let solo = solo_agent(w.tx, agent)?;
            if blocking {
                if t.status == TicketStatus::InProgress {
                    transition(w, &mut t, &ctx.actor, TicketStatus::Pending, &Guards { open_blocking: true, ..Default::default() })?;
                }
                set_agent_status(w, agent, AgentStatus::Waiting)?;
                if solo {
                    // A Solo agent's questions go to the user (it is its own Lead).
                    w.emit_as("system", DomainEvent::NeedsUser { ticket: Some(t.id), reason: format!("{} asks a blocking question", t.key) })?;
                }
            }
            Ok(Outcome {
                result: serde_json::json!({ "comment_id": c.id, "blocking": blocking, "routed_to": if solo { "user" } else { "lead" } }),
                run: if blocking { RunEffect::Park } else { RunEffect::Continue },
                ops: vec![],
            })
        })
    }

    pub(super) fn answer_question(&self, ctx: &CallCtx, a: AnswerQuestion) -> Result<Outcome> {
        let pid = ctx.project;
        self.commit(ctx, |w| {
            let team = require_lead(w, pid, &ctx.actor)?;
            let mut t = ticket_by_key(w, pid, &a.ticket_id)?;
            let q = comments::find(w.tx, a.reply_to)?
                .ok_or_else(|| invalid(format!("comment {} does not exist", a.reply_to)))?;
            if q.ticket != t.id || q.kind != CommentKind::Question {
                return Err(invalid("reply_to must be a Question on this ticket"));
            }
            if solo_self(team.as_ref(), &ctx.actor, &q) {
                return Err(denied(
                    "a Solo agent's own questions go to the user: wait for the answer (the ticket resumes then)",
                ));
            }
            if q.resolved {
                return Err(invalid("this question is already answered"));
            }
            if let (Some(i), CommentBody::Question(qq)) = (a.chosen_option, &q.body) {
                if i >= qq.options.len() {
                    return Err(invalid(
                        "chosen_option is not an index into the question's options",
                    ));
                }
            }
            if a.escalate {
                // The Lead hands the question to the user; it stays open (the user answers it) and leaves the inbox.
                let Actor::Lead(lead) = ctx.actor else {
                    return Err(invalid("only the Lead escalates a question; answer it instead"));
                };
                let note = add_comment(w, &t, Author::Agent(lead), Some(q.id), CommentBody::Text { text: format!("Escalated to the user: {}", a.answer.trim()) })?;
                w.emit(DomainEvent::NeedsUser { ticket: Some(t.id), reason: format!("{}: the Lead escalated a question to you", t.key) })?;
                return Ok(Outcome::ok(serde_json::json!({ "comment_id": note.id, "escalated": true, "ticket_status": t.status })));
            }
            let ans = add_comment(
                w,
                &t,
                author_of(&ctx.actor),
                Some(q.id),
                CommentBody::Answer(Answer {
                    resolution: a.answer,
                    chosen_option: a.chosen_option,
                }),
            )?;
            comments::set_resolved(w.tx, q.id, true)?;
            w.emit(DomainEvent::CommentResolved {
                comment: q.id,
                ticket: t.id,
            })?;
            unblock_if_clear(w, &mut t)?;
            Ok(Outcome::ok(
                serde_json::json!({ "comment_id": ans.id, "ticket_status": t.status }),
            ))
        })
    }

    /// Where the user's answer to an open Question goes (ADR-122). A Solo agent is the one the user talks to: the
    /// answer goes on its ticket. In a team the user talks to the Lead only: the answer becomes a message to the
    /// Lead, who answers on the ticket — the worker hears it from the Lead, never from the user.
    pub fn route_user_answer(
        &self,
        project: ProjectId,
        key: &TicketKey,
        question: CommentId,
        answer: &str,
        chosen_option: Option<usize>,
    ) -> Result<UserAnswer> {
        let team = self
            .store
            .read(|c| agents::teams_of_project(c, project))?
            .into_iter()
            .next();
        if team.as_ref().is_none_or(|t| t.solo) {
            return Ok(UserAnswer::Answered(self.user_answer(
                project,
                key,
                question,
                answer,
                chosen_option,
            )?));
        }
        let t = self.store.read(|c| tickets::get_by_key(c, project, key))?;
        let q = self
            .store
            .read(|c| comments::find(c, question))?
            .ok_or_else(|| invalid(format!("comment {question} does not exist")))?;
        let CommentBody::Question(qq) = &q.body else {
            return Err(invalid("this comment is not a question"));
        };
        if q.ticket != t.id {
            return Err(invalid("this question is not on that ticket"));
        }
        if q.resolved {
            return Err(invalid("this question is already answered"));
        }
        let mut answer = answer.trim().to_owned();
        if let Some(o) = chosen_option.and_then(|i| qq.options.get(i)) {
            answer = if answer.is_empty() {
                o.label.clone()
            } else {
                format!("{} — {answer}", o.label)
            };
        }
        if answer.is_empty() {
            return Err(invalid("the answer is empty"));
        }
        let message = if self.config().app.language == "vi" {
            format!(
                "Trả lời câu hỏi trên {key} mà bạn chuyển cho tôi:\n> {}\n\n{answer}\n\n(Hãy trả lời câu hỏi {question} trên ticket cho team.)",
                qq.question.trim()
            )
        } else {
            format!(
                "My answer to the question on {key} you passed to me:\n> {}\n\n{answer}\n\n(Answer question {question} on the ticket for the team.)",
                qq.question.trim()
            )
        };
        Ok(UserAnswer::ToLead(message))
    }

    /// The user answers an open Question on its ticket (Solo agents; tests and scripted evaluations).
    pub fn user_answer(
        &self,
        project: ProjectId,
        key: &TicketKey,
        question: CommentId,
        answer: &str,
        chosen_option: Option<usize>,
    ) -> Result<Outcome> {
        let ctx = CallCtx::user(project);
        self.answer_question(
            &ctx,
            AnswerQuestion {
                ticket_id: key.clone(),
                reply_to: question,
                answer: answer.to_owned(),
                chosen_option,
                escalate: false,
            },
        )
    }
}
