//! The Lead's inbox (team mode): what waits for the Lead, most urgent first — blocking questions and permission
//! requests, then plan items without tickets and tickets without a worker, then non-blocking questions. Reviews of
//! Resolved tickets have their own runs. The blocking-question SLA escalates to the user when the Lead is late.

use std::collections::HashSet;

use agenium_core::*;
use agenium_store::core::{comments, plans};
use serde::Serialize;

use crate::app::App;
use crate::engine::{allowlist, in_allowlist};
use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum InboxKind {
    BlockingQuestion,
    Permission,
    /// The spec is approved and there is no plan: create plan v1 (from the planner's draft when the team has one).
    Planning,
    /// Approved plan items that have no ticket yet.
    Plan,
    /// New tickets without a worker, or Reopened tickets whose worker cannot take work.
    Assign,
    Question,
}

#[derive(Debug, Clone, Serialize)]
pub struct InboxItem {
    pub kind: InboxKind,
    pub ticket: Option<TicketKey>,
    /// The Question or Permission comment.
    pub comment: Option<CommentId>,
    pub created_at: String,
    /// What the Lead has to decide, for the run's task text.
    pub text: String,
}

/// Note posted under a question when its SLA ran out (posted once per question).
const SLA_NOTE: &str = "SLA exceeded:";

impl App {
    /// The inbox of the project's Lead; empty without a team. A Solo agent's inbox has only the items that move the
    /// work on (plan, tickets to create or assign: ADR-63); its questions go to the user.
    pub fn lead_inbox(&self, project: ProjectId) -> Result<Vec<InboxItem>> {
        let Some((team, members)) = self.team_of_project(project)? else {
            return Ok(vec![]);
        };
        let Some(lead) = team.lead else {
            return Ok(vec![]);
        };
        let p = self.project(project)?;
        let list = allowlist(self, &p);
        let tickets = self.tickets(project)?;
        let who = |a: &Author| match a {
            Author::Agent(id) => members
                .iter()
                .find(|m| m.id == *id)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| id.to_string()),
            other => other.to_string(),
        };
        let mut out = Vec::new();
        for t in tickets.iter().filter(|t| !t.status.is_terminal()) {
            let cs = self.store.read(|c| comments::for_ticket(c, t.id))?;
            // Questions the Lead escalated to the user are the user's now.
            let escalated: HashSet<CommentId> = cs
                .iter()
                .filter(|c| {
                    c.author == Author::Agent(lead) && matches!(c.body, CommentBody::Text { .. })
                })
                .filter_map(|c| c.reply_to)
                .collect();
            for c in &cs {
                match &c.body {
                    CommentBody::Question(q)
                        if !c.resolved
                            && c.author != Author::Agent(lead)
                            && !escalated.contains(&c.id) =>
                    {
                        out.push(InboxItem {
                            kind: if q.blocking {
                                InboxKind::BlockingQuestion
                            } else {
                                InboxKind::Question
                            },
                            ticket: Some(t.key.clone()),
                            comment: Some(c.id),
                            created_at: c.created_at.clone(),
                            text: format!(
                                "{} question {} on {} ({:?}, {}): {}{}{}",
                                if q.blocking {
                                    "Blocking"
                                } else {
                                    "Non-blocking"
                                },
                                c.id,
                                t.key,
                                q.category,
                                who(&c.author),
                                q.question,
                                if q.context.is_empty() {
                                    String::new()
                                } else {
                                    format!("\n  context: {}", q.context)
                                },
                                if q.options.is_empty() {
                                    String::new()
                                } else {
                                    format!(
                                        "\n  options: {}",
                                        q.options
                                            .iter()
                                            .enumerate()
                                            .map(|(i, o)| format!("[{i}] {}", o.label))
                                            .collect::<Vec<_>>()
                                            .join("; ")
                                    )
                                }
                            ),
                        });
                    }
                    CommentBody::Permission {
                        request,
                        outcome: None,
                        ..
                    } if in_allowlist(request, &list) => {
                        out.push(InboxItem {
                            kind: InboxKind::Permission,
                            ticket: Some(t.key.clone()),
                            comment: Some(c.id),
                            created_at: c.created_at.clone(),
                            text: format!(
                                "Permission request {} on {} ({:?}, inside the project allowlist): {}",
                                c.id,
                                t.key,
                                request.kind,
                                serde_json::to_string(request).unwrap_or_default()
                            ),
                        });
                    }
                    _ => {}
                }
            }
            let worker_cannot_work = |a: AgentId| {
                members
                    .iter()
                    .find(|m| m.id == a)
                    .is_none_or(|m| !m.status.assignable())
            };
            let unassigned = t.status == TicketStatus::New && t.assignee.is_none();
            let stranded =
                t.status == TicketStatus::Reopened && t.assignee.is_some_and(worker_cannot_work);
            if unassigned || stranded {
                out.push(InboxItem {
                    kind: InboxKind::Assign,
                    ticket: Some(t.key.clone()),
                    comment: None,
                    created_at: t.updated_at.clone(),
                    text: if unassigned {
                        format!("{} ({}) has no worker: assign_ticket to a matching team member", t.key, t.title)
                    } else {
                        format!("{} ({}) was reopened but its worker cannot take work: assign it to another member", t.key, t.title)
                    },
                });
            }
        }
        let (spec, any_plan) = self.store.read(|c| {
            Ok((
                plans::latest_spec(c, project)?,
                plans::latest_plan(c, project)?.is_some(),
            ))
        })?;
        if let Some(spec) = spec.filter(|s| s.locked_at.is_some() && !any_plan) {
            let text = match (self.planner(project)?, self.plan_draft(project, spec.version)?) {
                (Some(p), Some(d)) => Some(format!(
                    "Spec v{v} is approved and your planner {} drafted plan v1. Review the draft against the spec and the team, adjust what you disagree with, then create_plan (you approve it: the user agreed on the spec).\nPlanner's rationale: {}\nDraft: {}",
                    p.name,
                    d.rationale,
                    serde_json::to_string(&d.plan()).unwrap_or_default(),
                    v = spec.version
                )),
                // The planner has not drafted yet: its run comes first.
                (Some(_), None) => None,
                (None, _) => Some(format!("Spec v{} is approved and has no plan yet: create_plan (you approve it: the user agreed on the spec).", spec.version)),
            };
            if let Some(mut text) = text {
                if !team.solo {
                    let roles: std::collections::BTreeSet<&str> = members
                        .iter()
                        .filter(|m| m.id != lead)
                        .map(|m| m.role.as_str())
                        .collect();
                    text.push_str(&format!(
                        "
You take no tickets: give every plan item one of your workers' roles ({}).",
                        roles.into_iter().collect::<Vec<_>>().join(", ")
                    ));
                }
                out.push(InboxItem {
                    kind: InboxKind::Planning,
                    ticket: None,
                    comment: None,
                    created_at: spec.locked_at.clone().unwrap_or_default(),
                    text,
                });
            }
        }
        if let Some(plan) = self.store.read(|c| plans::current_plan(c, project))? {
            let planned: HashSet<&str> = tickets.iter().map(|t| t.plan_item.as_str()).collect();
            let missing: Vec<String> = plan
                .body
                .items
                .iter()
                .filter(|i| !planned.contains(i.id.as_str()))
                .map(|i| format!("{} {} (role {})", i.id, i.title, i.role))
                .collect();
            if !missing.is_empty() {
                out.push(InboxItem {
                    kind: InboxKind::Plan,
                    ticket: None,
                    comment: None,
                    created_at: String::new(),
                    text: format!(
                        "Approved plan v{} has items without tickets: {}. create_ticket for each (respect depends_on between items), then assign_ticket.",
                        plan.version,
                        missing.join("; ")
                    ),
                });
            }
        }
        if team.solo {
            out.retain(|i| {
                matches!(
                    i.kind,
                    InboxKind::Planning | InboxKind::Plan | InboxKind::Assign
                )
            });
        }
        out.sort_by(|a, b| {
            a.kind
                .cmp(&b.kind)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });
        Ok(out)
    }

    /// Blocking questions of workers open longer than the SLA get a System note and go to the user (once each).
    /// Returns how many were escalated now.
    pub fn check_sla(
        &self,
        project: ProjectId,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<usize> {
        let minutes = i64::from(self.config().runtime.blocking_question_sla_minutes.max(1));
        let mut late = Vec::new();
        for item in self.lead_inbox(project)? {
            if item.kind != InboxKind::BlockingQuestion {
                continue;
            }
            let Ok(at) = chrono::DateTime::parse_from_rfc3339(&item.created_at) else {
                continue;
            };
            if now.signed_duration_since(at) < chrono::Duration::minutes(minutes) {
                continue;
            }
            let (Some(key), Some(q)) = (item.ticket, item.comment) else {
                continue;
            };
            let t = self.ticket(project, &key)?;
            let noted = self.store.read(|c| comments::for_ticket(c, t.id))?.iter().any(|c| {
                c.reply_to == Some(q) && matches!(&c.body, CommentBody::System { text } if text.starts_with(SLA_NOTE))
            });
            if !noted {
                late.push((t, q));
            }
        }
        let n = late.len();
        if n > 0 {
            self.store.commit(None, "system", |w| -> Result<()> {
                for (t, q) in &late {
                    comments::insert(
                        w.tx,
                        CommentId::new(),
                        t.id,
                        Author::System,
                        Some(*q),
                        &CommentBody::System {
                            text: format!("{SLA_NOTE} the blocking question {q} waited more than {minutes} min for the Lead; it goes to the user too (either can answer)."),
                        },
                        &now_iso(),
                    )?;
                    w.emit(DomainEvent::NeedsUser {
                        ticket: Some(t.id),
                        reason: format!("{}: a blocking question passed its {minutes} min SLA", t.key),
                    })?;
                }
                Ok(())
            })?;
        }
        Ok(n)
    }
}
