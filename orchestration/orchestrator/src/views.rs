//! Read models for the user: the board (Kanban columns by status) and the audit trail (M3-5). The headless CLI
//! prints them now; the UI reads the same structures later.

use std::collections::HashMap;
use std::str::FromStr;

use agenium_core::*;
use agenium_store::core::{comments, events, plans, runs};
use serde::Serialize;

use crate::app::App;
use crate::Result;

/// Board columns, in workflow order.
pub const COLUMNS: [TicketStatus; 8] = [
    TicketStatus::New,
    TicketStatus::Assigned,
    TicketStatus::InProgress,
    TicketStatus::Pending,
    TicketStatus::Resolved,
    TicketStatus::Reopened,
    TicketStatus::Closed,
    TicketStatus::Cancelled,
];

#[derive(Debug, Clone, Serialize)]
pub struct Card {
    pub key: TicketKey,
    pub title: String,
    pub kind: TicketKind,
    pub priority: u8,
    pub plan_item: String,
    pub assignee: Option<String>,
    /// Dependencies not satisfied yet (not Closed, nor Resolved + approved (+ merged)).
    pub blocked_by: Vec<TicketKey>,
    pub open_questions: usize,
    pub approved: bool,
    pub merged: bool,
    pub escalated: bool,
    pub reopen_count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Column {
    pub status: TicketStatus,
    pub cards: Vec<Card>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditEntry {
    pub at: String,
    /// `user`, `system` or the agent's name.
    pub actor: String,
    pub ticket: Option<TicketKey>,
    /// Event kind for committed changes, the protocol tool for refused calls.
    pub action: String,
    pub intent_id: Option<String>,
    /// `false`: an agent's call that the orchestrator refused (nothing changed).
    pub accepted: bool,
    pub detail: serde_json::Value,
}

/// Discovery state for the user: the latest spec and plan with their approval, and progress per plan item.
#[derive(Debug, Clone, Serialize)]
pub struct PlanView {
    pub spec: Option<plans::SpecRow>,
    /// Latest plan version (may wait for approval) and the approved one it replaces, if different.
    pub plan: Option<plans::PlanRow>,
    pub current_plan_version: Option<u32>,
    pub items: Vec<ItemProgress>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ItemProgress {
    pub id: String,
    pub milestone: String,
    pub title: String,
    /// Every ticket of the item ended and at least one Closed (the item is ticked).
    pub done: bool,
    pub tickets: Vec<(TicketKey, TicketStatus)>,
}

/// Where a project's team stands (ADR-63): it works on its own from the agreed requirements to the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RunState {
    /// No agreed (locked) requirement spec yet: the Lead and the user are still talking.
    Agreeing,
    /// The team works: plan, tickets, reviews, round after round.
    Running,
    /// Stopped by the user.
    Paused,
    /// Every plan item is done and no ticket is open.
    Done,
}

impl App {
    /// The part of the agreed spec a ticket serves: the goals, and the requirements its plan item traces to with
    /// their acceptance criteria. Workers cannot read the spec themselves (it is not in their worktree).
    pub fn ticket_requirements(&self, project: ProjectId, t: &Ticket) -> Result<String> {
        let (plan, spec) = self.store.read(|c| {
            let plan = plans::current_plan(c, project)?;
            let spec = match &plan {
                Some(p) => plans::spec(c, project, p.body.spec_version)?,
                None => None,
            };
            Ok((plan, spec))
        })?;
        let (Some(plan), Some(spec)) = (plan, spec) else {
            return Ok(String::new());
        };
        let Some(item) = plan.body.items.iter().find(|i| i.id == t.plan_item) else {
            return Ok(String::new());
        };
        let mut s = format!(
            "\n## What this ticket serves (spec v{}, plan item {}: {})\nGoals: {}\n",
            spec.version,
            item.id,
            item.title,
            spec.body.goals.join("; ")
        );
        for r in spec
            .body
            .requirements
            .iter()
            .filter(|r| item.trace_to.contains(&r.id))
        {
            s.push_str(&format!("- {}: {}\n", r.id, r.text));
            for a in &r.acceptance {
                s.push_str(&format!("  - {}: {}\n", a.id, a.text));
            }
        }
        if !spec.body.constraints.is_empty() {
            s.push_str(&format!(
                "Constraints: {}\n",
                spec.body.constraints.join("; ")
            ));
        }
        Ok(s)
    }

    /// The project's run state, from its status, its spec and its plan.
    pub fn run_state(&self, project: ProjectId) -> Result<RunState> {
        use agenium_store::core::projects::ProjectStatus;
        let p = self.project(project)?;
        if matches!(p.status, ProjectStatus::Paused | ProjectStatus::Cancelled) {
            return Ok(RunState::Paused);
        }
        let v = self.plan_view(project)?;
        let agreed = v.spec.as_ref().is_some_and(|s| s.locked_at.is_some());
        if !agreed && v.plan.is_none() {
            return Ok(RunState::Agreeing);
        }
        let spec_open = v.spec.as_ref().is_some_and(|s| s.locked_at.is_none());
        let plan_open = v.plan.as_ref().is_some_and(|p| p.approved_by.is_none());
        let open_tickets = self
            .tickets(project)?
            .iter()
            .any(|t| !t.status.is_terminal());
        let all_done = !v.items.is_empty() && v.items.iter().all(|i| i.done);
        if all_done && !open_tickets && !spec_open && !plan_open {
            return Ok(RunState::Done);
        }
        Ok(RunState::Running)
    }

    pub fn plan_view(&self, project: ProjectId) -> Result<PlanView> {
        let (spec, plan, current) = self.store.read(|c| {
            Ok((
                plans::latest_spec(c, project)?,
                plans::latest_plan(c, project)?,
                plans::current_plan(c, project)?.map(|p| p.version),
            ))
        })?;
        let tickets = self.tickets(project)?;
        let items = plan
            .as_ref()
            .map(|p| {
                p.body
                    .items
                    .iter()
                    .map(|i| {
                        let mine: Vec<&Ticket> =
                            tickets.iter().filter(|t| t.plan_item == i.id).collect();
                        ItemProgress {
                            id: i.id.clone(),
                            milestone: i.milestone.clone(),
                            title: i.title.clone(),
                            done: !mine.is_empty()
                                && mine.iter().all(|t| t.status.is_terminal())
                                && mine.iter().any(|t| t.status == TicketStatus::Closed),
                            tickets: mine.iter().map(|t| (t.key.clone(), t.status)).collect(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(PlanView {
            spec,
            plan,
            current_plan_version: current,
            items,
        })
    }

    /// The project's tickets in board columns; inside a column by priority, then age.
    pub fn board(&self, project: ProjectId) -> Result<Vec<Column>> {
        let tickets = self.tickets(project)?;
        let names: HashMap<AgentId, String> =
            self.agents()?.into_iter().map(|a| (a.id, a.name)).collect();
        let by_key: HashMap<&TicketKey, &Ticket> = tickets.iter().map(|t| (&t.key, t)).collect();
        let open_q: HashMap<TicketId, usize> = self.store.read(|c| {
            let mut m = HashMap::new();
            for t in &tickets {
                let n = comments::for_ticket(c, t.id)?
                    .iter()
                    .filter(|c| c.kind == CommentKind::Question && !c.resolved)
                    .count();
                m.insert(t.id, n);
            }
            Ok(m)
        })?;
        let mut cols: Vec<Column> = COLUMNS
            .iter()
            .map(|s| Column {
                status: *s,
                cards: vec![],
            })
            .collect();
        for t in &tickets {
            let blocked_by = t
                .depends_on
                .iter()
                .filter(|k| by_key.get(k).is_none_or(|d| !dep_satisfied(d)))
                .cloned()
                .collect();
            let card = Card {
                key: t.key.clone(),
                title: t.title.clone(),
                kind: t.kind,
                priority: t.priority,
                plan_item: t.plan_item.clone(),
                assignee: t
                    .assignee
                    .map(|a| names.get(&a).cloned().unwrap_or_else(|| a.to_string())),
                blocked_by,
                open_questions: open_q.get(&t.id).copied().unwrap_or(0),
                approved: t.approved,
                merged: t.merged,
                escalated: t.escalated,
                reopen_count: t.reopen_count,
            };
            if let Some(c) = cols.iter_mut().find(|c| c.status == t.status) {
                c.cards.push(card);
            }
        }
        let age: HashMap<&TicketKey, &str> = tickets
            .iter()
            .map(|t| (&t.key, t.created_at.as_str()))
            .collect();
        for c in &mut cols {
            c.cards.sort_by(|a, b| {
                a.priority
                    .cmp(&b.priority)
                    .then_with(|| age[&a.key].cmp(age[&b.key]))
            });
        }
        Ok(cols)
    }

    /// Audit trail (who did what, the originating intent, the check result, when): committed events plus the
    /// protocol calls the orchestrator refused. For one ticket, or the whole project when `key` is `None`.
    pub fn audit(&self, project: ProjectId, key: Option<&TicketKey>) -> Result<Vec<AuditEntry>> {
        let tickets = match key {
            Some(k) => vec![self.ticket(project, k)?],
            None => self.tickets(project)?,
        };
        let keys: HashMap<TicketId, TicketKey> =
            tickets.iter().map(|t| (t.id, t.key.clone())).collect();
        let names: HashMap<AgentId, String> =
            self.agents()?.into_iter().map(|a| (a.id, a.name)).collect();
        let label_of: HashMap<String, &String> = names
            .iter()
            .map(|(id, n)| (format!("agent:{id}"), n))
            .collect();
        let actor = |label: &str| {
            label_of
                .get(label)
                .map(|n| (*n).clone())
                .unwrap_or_else(|| label.to_owned())
        };
        let mut out = Vec::new();
        let records = self.store.read(|c| {
            let mut v = Vec::new();
            if key.is_some() {
                for t in &tickets {
                    v.extend(events::for_ticket(c, t.id)?);
                }
            } else {
                // Project-level events (spec, plan) carry the project in their payload.
                let pid = project.to_string();
                for e in events::since(c, 0, usize::MAX >> 1)? {
                    let mine = match e.ticket {
                        Some(t) => keys.contains_key(&t),
                        None => {
                            e.payload.get("project").and_then(|p| p.as_str()) == Some(pid.as_str())
                        }
                    };
                    if mine {
                        v.push(e);
                    }
                }
            }
            Ok(v)
        })?;
        for e in records {
            out.push(AuditEntry {
                at: e.at,
                actor: actor(&e.actor),
                ticket: e.ticket.and_then(|t| keys.get(&t).cloned()),
                action: e.kind,
                intent_id: e.intent_id,
                accepted: true,
                detail: e.payload,
            });
        }
        // Refused protocol calls live in the run logs (they changed nothing, so the event log has no trace).
        for t in &tickets {
            for r in self.store.read(|c| runs::for_ticket(c, t.id))? {
                for l in self.store.read(|c| runs::log_entries(c, r.id))? {
                    let tool = l.data["tool"].as_str().unwrap_or_default();
                    if l.kind != "tool"
                        || l.data["ok"] != false
                        || ProtocolTool::from_str(tool).is_err()
                    {
                        continue;
                    }
                    out.push(AuditEntry {
                        at: l.at,
                        actor: names.get(&r.agent).cloned().unwrap_or_else(|| r.agent.to_string()),
                        ticket: Some(t.key.clone()),
                        action: tool.to_owned(),
                        intent_id: Some(format!("{}:{}", r.id, l.n)),
                        accepted: false,
                        detail: serde_json::json!({ "args": l.data["args"], "error": l.data["result"] }),
                    });
                }
            }
        }
        out.sort_by(|a, b| a.at.cmp(&b.at));
        Ok(out)
    }
}

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
    pub text: String,
    pub at: String,
}

impl App {
    /// Everything that waits for the user in `project` (derived from the database, nothing to acknowledge).
    pub fn needs_you(&self, project: ProjectId) -> Result<Vec<NeedItem>> {
        let mut out = Vec::new();
        let need = |kind, ticket: Option<&Ticket>, text: String, at: &str| NeedItem {
            kind,
            project,
            ticket: ticket.map(|t| t.key.clone()),
            comment: None,
            version: None,
            agent: None,
            text,
            at: at.to_owned(),
        };
        let team = self.team_of_project(project)?;
        if let Some((_, members)) = &team {
            for a in members.iter().filter(|a| !a.status.assignable()) {
                out.push(NeedItem {
                    agent: Some(a.name.clone()),
                    ..need(
                        NeedKind::AgentBlocked,
                        None,
                        format!("{} is {}: change its model or add credit", a.name, a.status),
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
                    format!("Requirement spec v{} waits for your approval", s.version),
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
                    format!(
                        "Plan v{} waits for your approval{}",
                        p.version,
                        p.reason
                            .as_deref()
                            .map(|r| format!(": {r}"))
                            .unwrap_or_default()
                    ),
                    "",
                )
            });
        }
        let lead_items: std::collections::HashSet<CommentId> = self
            .lead_inbox(project)?
            .into_iter()
            .filter_map(|i| i.comment)
            .collect();
        let review_gate = self.project(project)?.review_before_merge();
        for t in self
            .tickets(project)?
            .iter()
            .filter(|t| !t.status.is_terminal())
        {
            let cs = self.store.read(|c| comments::for_ticket(c, t.id))?;
            if t.escalated {
                out.push(need(
                    NeedKind::Escalation,
                    Some(t),
                    format!("{} was reopened {} times", t.key, t.reopen_count),
                    &t.updated_at,
                ));
            }
            if let Some(c) = cs.iter().rev().find(|c| matches!(&c.body, CommentBody::System { text } if text.starts_with("Stall: escalated"))) {
                // Only while no run succeeded since (a later good run would have moved the ticket on).
                if t.status != TicketStatus::Resolved || !t.approved {
                    out.push(need(NeedKind::Escalation, Some(t), format!("{}: runs keep stopping; look at the run log", t.key), &c.created_at));
                }
            }
            for c in &cs {
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
                            format!(
                                "{}: {:?} permission requested: {}",
                                t.key, request.kind, request.reason
                            ),
                            &c.created_at,
                        )
                    }),
                    CommentBody::Question(q) if !c.resolved && !lead_items.contains(&c.id) => out
                        .push(NeedItem {
                            comment: Some(c.id),
                            ..need(
                                NeedKind::Question,
                                Some(t),
                                format!("{}: {}", t.key, q.question),
                                &c.created_at,
                            )
                        }),
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
                    format!(
                        "{} is approved and waits for your review before merge",
                        t.key
                    ),
                    &t.updated_at,
                ));
            }
        }
        out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.at.cmp(&b.at)));
        Ok(out)
    }
}

/// Markdown rendering of an audit trail (export for people; JSON Lines is the machine form).
pub fn audit_markdown(title: &str, entries: &[AuditEntry]) -> String {
    let mut s = format!("# Audit trail: {title}\n\n| When (UTC) | Who | Ticket | Action | Result | Detail |\n|---|---|---|---|---|---|\n");
    for e in entries {
        let detail: String = e.detail.to_string().chars().take(300).collect();
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | `{}` |\n",
            e.at,
            e.actor,
            e.ticket.as_ref().map(|k| k.to_string()).unwrap_or_default(),
            e.action,
            if e.accepted { "applied" } else { "refused" },
            detail.replace('|', "\\|").replace('`', "'")
        ));
    }
    s
}
