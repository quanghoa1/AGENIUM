//! What waits for the user ("Needs you"): derived from the database each time, nothing to acknowledge. Each item
//! carries its English sentence (`text`) and the same sentence as a template id and its values (`words`), which the
//! interface puts in its own language; what an agent wrote (a question, a reason) stays as written.

use agenium_core::*;
use agenium_store::core::{comments, plans};
use serde::Serialize;

use crate::app::App;
use crate::Result;

/// What waits for the user ("Needs you" on the dashboard), most urgent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum NeedKind {
    /// An agent's provider reports exhausted quota/credit, or it is offline: nothing runs until the user acts.
    AgentBlocked,
    /// Runs keep stopping (stall-nudges used up), or the ticket was reopened too often.
    Escalation,
    SpecApproval,
    PlanApproval,
    /// A permission request outside the project allowlist.
    Permission,
    /// A question for the user: a Solo agent's, one the Lead escalated, or one past its SLA.
    Question,
    /// "Review before merge": an approved code ticket waits for the user's merge.
    MergeReview,
    /// The Lead proposes a role its team lacks (decision 3l): approve (an agent joins) or decline.
    RoleProposal,
    /// An agent proposes a new skill (ADR-152): open it in the skill editor and save it into the library, or decline.
    SkillProposal,
    /// The Lead cancelled a ticket and no ticket carries its work on (ADR-147): waive it, or ask the Lead for it.
    CancelledWork,
}

#[derive(Debug, Clone, Serialize)]
pub struct NeedItem {
    pub kind: NeedKind,
    pub project: ProjectId,
    pub ticket: Option<TicketKey>,
    /// The comment to act on (question, permission request).
    pub comment: Option<CommentId>,
    /// Spec or plan version to approve.
    pub version: Option<u32>,
    pub agent: Option<String>,
    /// The agent to resume (`AgentBlocked`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// The role proposal to decide (`RoleProposal`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proposal: Option<String>,
    /// The item in English (logs, the CLI, an older interface).
    pub text: String,
    /// The same sentence for the interface to put in its own language.
    pub words: NeedWords,
    pub at: String,
}

/// A "needs you" sentence as a template id and the values that fill its `{0}`, `{1}`…: names, numbers, and what an
/// agent wrote (a question, a reason), which stays as written. The interface keeps each template as
/// `need.words.<id>` in each of its languages; `SENTENCES` holds the English ones.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NeedWords {
    pub id: &'static str,
    pub args: Vec<String>,
}

/// The English sentence of each template id.
pub const SENTENCES: &[(&str, &str)] = &[
    ("quota", "{0} ran out of quota or credit on {1}: it goes on by itself once the quota is back (tried again when the vendor's window resets, or from 30 minutes on); add credit, resume it now or give it another model; it goes on from where it stopped"),
    ("offline", "{0} cannot use {1} (its provider unreachable, or its CLI missing or logged out): fix it or give it another model, then resume it; it goes on from where it stopped"),
    ("spec", "Requirement spec v{0} waits for your approval"),
    ("plan", "Plan v{0} waits for your approval"),
    ("planWhy", "Plan v{0} waits for your approval: {1}"),
    ("reopened", "{0} was reopened {1} times"),
    ("stalled", "{0}: runs keep stopping; look at the run log"),
    ("permission", "{0}: {1} permission requested: {2}"),
    ("question", "{0}: {1}"),
    ("mergeReview", "{0} is approved and waits for your review before merge"),
    ("role", "The Lead proposes a new role {0} ({1}): {2}"),
    ("roleModel", "The Lead proposes a new role {0} ({1}): {2} Suggested model: {3}."),
    ("skill", "{0} proposes a new skill {1} from a lesson that held in {2}: {3}. Reason: {4}"),
    ("skillClose", "{0} proposes a new skill {1} from a lesson that held in {2}: {3}. Reason: {4} Close to library skill {5}: merge rather than add?"),
    ("cancelled", "The Lead cancelled {0} ({1}) and no ticket carries its work on: waive it if it is not needed, or ask the Lead for it"),
];

impl NeedWords {
    fn new(id: &'static str, args: Vec<String>) -> Self {
        Self { id, args }
    }

    /// The sentence in English.
    pub fn english(&self) -> String {
        let template = SENTENCES
            .iter()
            .find(|(id, _)| *id == self.id)
            .map(|(_, t)| *t)
            .unwrap_or("{0}");
        // One pass over the template: a value that contains `{1}` is not filled again.
        let mut out = String::with_capacity(template.len() + 64);
        let mut rest = template;
        while let Some(at) = rest.find('{') {
            out.push_str(&rest[..at]);
            let tail = &rest[at + 1..];
            match tail
                .find('}')
                .and_then(|end| tail[..end].parse::<usize>().ok().map(|i| (i, end)))
            {
                Some((i, end)) => {
                    out.push_str(self.args.get(i).map(String::as_str).unwrap_or(""));
                    rest = &tail[end + 1..];
                }
                None => {
                    out.push('{');
                    rest = tail;
                }
            }
        }
        out.push_str(rest);
        out
    }
}

/// "X (duplicate), Y (near)" for a proposal's similar library skills (ADR-152), or nothing.
fn close_to(similar: &serde_json::Value) -> String {
    let names: Vec<String> = similar
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            Some(format!(
                "{} ({})",
                s["name"].as_str()?,
                s["level"].as_str().unwrap_or("near")
            ))
        })
        .collect();
    names.join(", ")
}

impl App {
    /// Everything that waits for the user in `project` (derived from the database, nothing to acknowledge).
    pub fn needs_you(&self, project: ProjectId) -> Result<Vec<NeedItem>> {
        let mut out = Vec::new();
        let need = |kind, ticket: Option<&Ticket>, words: NeedWords, at: &str| NeedItem {
            kind,
            project,
            ticket: ticket.map(|t| t.key.clone()),
            comment: None,
            version: None,
            agent: None,
            agent_id: None,
            proposal: None,
            text: words.english(),
            words,
            at: at.to_owned(),
        };
        let team = self.team_of_project(project)?;
        if let Some((_, members)) = &team {
            // The agent waits for the user (ADR-132): what is wrong with its model, and that resuming goes on from
            // where it stopped (its ticket, its conversation).
            for a in members.iter().filter(|a| !a.status.assignable()) {
                let model = format!("{}/{}", a.model.provider, a.model.model);
                let id = if a.status == AgentStatus::QuotaExhausted {
                    "quota"
                } else {
                    "offline"
                };
                out.push(NeedItem {
                    agent: Some(a.name.clone()),
                    agent_id: Some(a.id),
                    ..need(
                        NeedKind::AgentBlocked,
                        None,
                        NeedWords::new(id, vec![a.name.clone(), model]),
                        "",
                    )
                });
            }
        }
        let (spec, plan) = self.store.read(|c| {
            Ok((
                plans::latest_spec(c, project)?,
                plans::latest_plan(c, project)?,
            ))
        })?;
        if let Some(s) = spec.filter(|s| s.locked_at.is_none()) {
            out.push(NeedItem {
                version: Some(s.version),
                ..need(
                    NeedKind::SpecApproval,
                    None,
                    NeedWords::new("spec", vec![s.version.to_string()]),
                    "",
                )
            });
        }
        if let Some(p) = plan.filter(|p| p.approved_by.is_none()) {
            out.push(NeedItem {
                version: Some(p.version),
                ..need(
                    NeedKind::PlanApproval,
                    None,
                    match p.reason.as_deref() {
                        Some(r) => {
                            NeedWords::new("planWhy", vec![p.version.to_string(), r.to_owned()])
                        }
                        None => NeedWords::new("plan", vec![p.version.to_string()]),
                    },
                    "",
                )
            });
        }
        // The comments of the open tickets, read once for the Lead's inbox and for the user's items.
        let by_ticket = self.store.read(|c| comments::of_open_tickets(c, project))?;
        let lead_items: std::collections::HashSet<CommentId> = self
            .inbox_from(project, &by_ticket)?
            .into_iter()
            .filter_map(|i| i.comment)
            .collect();
        let review_gate = self.project(project)?.review_before_merge();
        for t in self
            .tickets(project)?
            .iter()
            .filter(|t| !t.status.is_terminal())
        {
            let cs = by_ticket.get(&t.id).map(Vec::as_slice).unwrap_or_default();
            // A blocking question past its SLA goes to the user too (either answers), though the Lead still has it.
            let past_sla: std::collections::HashSet<CommentId> = cs
                .iter()
                .filter(|c| matches!(&c.body, CommentBody::System { text } if text.starts_with(crate::inbox::SLA_NOTE)))
                .filter_map(|c| c.reply_to)
                .collect();
            if t.escalated {
                out.push(need(
                    NeedKind::Escalation,
                    Some(t),
                    NeedWords::new(
                        "reopened",
                        vec![t.key.to_string(), t.reopen_count.to_string()],
                    ),
                    &t.updated_at,
                ));
            }
            if let Some(c) = cs.iter().rev().find(|c| matches!(&c.body, CommentBody::System { text } if text.starts_with("Stall: escalated"))) {
                // Only while no run succeeded since (a later good run would have moved the ticket on).
                if t.status != TicketStatus::Resolved || !t.approved {
                    out.push(need(
                        NeedKind::Escalation,
                        Some(t),
                        NeedWords::new("stalled", vec![t.key.to_string()]),
                        &c.created_at,
                    ));
                }
            }
            for c in cs {
                match &c.body {
                    CommentBody::Permission {
                        request,
                        outcome: None,
                        ..
                    } if !lead_items.contains(&c.id) => out.push(NeedItem {
                        comment: Some(c.id),
                        ..need(
                            NeedKind::Permission,
                            Some(t),
                            NeedWords::new(
                                "permission",
                                vec![
                                    t.key.to_string(),
                                    format!("{:?}", request.kind),
                                    request.reason.clone(),
                                ],
                            ),
                            &c.created_at,
                        )
                    }),
                    CommentBody::Question(q)
                        if !c.resolved
                            && (!lead_items.contains(&c.id) || past_sla.contains(&c.id)) =>
                    {
                        out.push(NeedItem {
                            comment: Some(c.id),
                            ..need(
                                NeedKind::Question,
                                Some(t),
                                NeedWords::new(
                                    "question",
                                    vec![t.key.to_string(), q.question.clone()],
                                ),
                                &c.created_at,
                            )
                        })
                    }
                    _ => {}
                }
            }
            if review_gate
                && t.status == TicketStatus::Resolved
                && t.approved
                && t.has_code
                && !t.merged
            {
                out.push(need(
                    NeedKind::MergeReview,
                    Some(t),
                    NeedWords::new("mergeReview", vec![t.key.to_string()]),
                    &t.updated_at,
                ));
            }
        }
        for p in self
            .role_proposals(project)?
            .into_iter()
            .filter(|p| p.status == agenium_store::core::roles::ProposalStatus::Pending)
        {
            out.push(NeedItem {
                proposal: Some(p.id.clone()),
                ..need(
                    NeedKind::RoleProposal,
                    None,
                    {
                        let mut args = vec![
                            p.body.role_id.to_string(),
                            p.body.title.trim().to_owned(),
                            p.body.description.trim().to_owned(),
                        ];
                        match p.body.model_hint.as_deref() {
                            Some(m) => {
                                args.push(m.to_owned());
                                NeedWords::new("roleModel", args)
                            }
                            None => NeedWords::new("role", args),
                        }
                    },
                    &p.created_at,
                )
            });
        }
        for p in self.skill_proposals(Some(project))? {
            let who = self.agent(p.agent).map(|a| a.name).unwrap_or_default();
            out.push(NeedItem {
                proposal: Some(p.id.clone()),
                agent: Some(who.clone()),
                ..need(
                    NeedKind::SkillProposal,
                    None,
                    {
                        let mut args = vec![
                            who.clone(),
                            p.name.clone(),
                            p.contexts
                                .iter()
                                .map(|c| c.rsplit(':').next().unwrap_or(c))
                                .collect::<Vec<_>>()
                                .join(", "),
                            p.lesson_text.trim().trim_end_matches('.').to_owned(),
                            p.reason.trim().to_owned(),
                        ];
                        let close = close_to(&p.similar);
                        if close.is_empty() {
                            NeedWords::new("skill", args)
                        } else {
                            args.push(close);
                            NeedWords::new("skillClose", args)
                        }
                    },
                    &p.created_at,
                )
            });
        }
        for keys in self.unaccounted_cancels(project)?.into_values() {
            for k in keys {
                let t = self.ticket(project, &k)?;
                out.push(need(
                    NeedKind::CancelledWork,
                    Some(&t),
                    NeedWords::new("cancelled", vec![t.key.to_string(), t.title.clone()]),
                    &t.updated_at,
                ));
            }
        }
        out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.at.cmp(&b.at)));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sentence_is_filled_once_from_its_values() {
        let w = NeedWords::new(
            "permission",
            vec![
                "SHOP-2".into(),
                "Network".into(),
                "npm install {1} from {0}".into(),
            ],
        );
        assert_eq!(
            w.english(),
            "SHOP-2: Network permission requested: npm install {1} from {0}"
        );
        assert_eq!(
            NeedWords::new("planWhy", vec!["3".into()]).english(),
            "Plan v3 waits for your approval: "
        );
        // No two sentences share an id.
        let ids: std::collections::HashSet<_> = SENTENCES.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids.len(), SENTENCES.len());
    }
}
