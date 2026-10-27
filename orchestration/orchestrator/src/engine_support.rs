//! Free helpers of the intent engine (engine.rs split): validation, comments, state transitions, permission records,
//! plan/scope checks, ticket insert. The intent methods stay in `engine.rs`; these are shared across them (and a few
//! live at `crate::engine::…` for other modules via a re-export there).

use std::collections::{BTreeSet, HashMap, HashSet};

use agenium_core::comment::PermissionOutcome;
use agenium_core::forms::*;
use agenium_core::graph::{find_cycle, scopes_overlap};
use agenium_core::*;
use agenium_store::core::plans::SpecRow;
use agenium_store::core::{agents, comments, plans, projects, runs, tickets, WriteCtx};

use crate::app::App;
use crate::{OrchError, Result};

pub(crate) fn denied(msg: impl Into<String>) -> OrchError {
    OrchError::Denied(msg.into())
}
pub(crate) fn invalid(msg: impl Into<String>) -> OrchError {
    OrchError::Invalid(msg.into())
}
pub(crate) fn ticket_by_key(
    w: &WriteCtx<'_, '_>,
    project: ProjectId,
    key: &TicketKey,
) -> Result<Ticket> {
    tickets::by_key(w.tx, project, key)?
        .ok_or_else(|| OrchError::NotFound(format!("ticket {key} does not exist in this project")))
}
/// The team of the project and whether `actor` is its Lead (Solo agent included). Users always may act as Lead.
pub(crate) fn require_lead(
    w: &WriteCtx<'_, '_>,
    project: ProjectId,
    actor: &Actor,
) -> Result<Option<agents::Team>> {
    let team = agents::teams_of_project(w.tx, project)?.into_iter().next();
    match actor {
        Actor::User => Ok(team),
        Actor::Lead(a) => {
            let t = team.ok_or_else(|| denied("the project has no team"))?;
            if t.lead == Some(*a) {
                Ok(Some(t))
            } else {
                Err(denied("only the team's Lead can do this"))
            }
        }
        _ => Err(denied("only the Lead or the user can do this")),
    }
}
/// The caller must be the assignee of `t` (worker tools).
pub(crate) fn require_assignee(t: &Ticket, actor: &Actor) -> Result<AgentId> {
    match actor {
        Actor::Worker(a) if t.assignee == Some(*a) => Ok(*a),
        Actor::Worker(_) => Err(denied(format!(
            "you are not the assignee of {}; workers act only on their own ticket",
            t.key
        ))),
        _ => Err(denied("this is a worker tool")),
    }
}
pub(crate) fn author_of(actor: &Actor) -> Author {
    match actor {
        Actor::Lead(a) | Actor::Worker(a) => Author::Agent(*a),
        Actor::User => Author::User,
        Actor::System => Author::System,
    }
}
pub(crate) fn add_comment(
    w: &mut WriteCtx<'_, '_>,
    t: &Ticket,
    author: Author,
    reply_to: Option<CommentId>,
    body: CommentBody,
) -> Result<Comment> {
    let c = comments::insert(
        w.tx,
        CommentId::new(),
        t.id,
        author,
        reply_to,
        &body,
        &now_iso(),
    )?;
    w.emit(DomainEvent::CommentAdded {
        comment: c.id,
        ticket: t.id,
        key: t.key.clone(),
        kind: c.kind,
    })?;
    Ok(c)
}
pub(crate) fn system_comment(
    w: &mut WriteCtx<'_, '_>,
    t: &Ticket,
    text: impl Into<String>,
) -> Result<Comment> {
    add_comment(
        w,
        t,
        Author::System,
        None,
        CommentBody::System { text: text.into() },
    )
}
pub(crate) fn set_agent_status(
    w: &mut WriteCtx<'_, '_>,
    agent: AgentId,
    to: AgentStatus,
) -> Result<()> {
    let from = agents::set_agent_status(w.tx, agent, to)?;
    if from != to {
        w.emit_as(
            "system",
            DomainEvent::AgentStatusChanged { agent, from, to },
        )?;
    }
    Ok(())
}
/// Checks and applies a status change, persisting the ticket and emitting events (escalation included).
pub(crate) fn transition(
    w: &mut WriteCtx<'_, '_>,
    t: &mut Ticket,
    actor: &Actor,
    to: TicketStatus,
    g: &Guards,
) -> Result<()> {
    check_transition(t, g, actor, to)?;
    let from = t.status;
    let fx = apply_transition(t, to);
    tickets::update(w.tx, t)?;
    w.emit_as(
        &actor.label(),
        DomainEvent::TicketStatusChanged {
            ticket: t.id,
            key: t.key.clone(),
            from,
            to,
        },
    )?;
    // The ticket's grants last until it ends, not until it is Resolved: a reopened ticket's branch still holds the
    // files its path grants allowed, and the scope check at the next submit_report reads them again.
    if to.is_terminal() {
        runs::revoke_ticket_grants(w.tx, t.id, &now_iso())?;
    }
    if fx.escalated_now {
        let reason = format!(
            "{} was reopened {} times; the user decides how to continue",
            t.key, t.reopen_count
        );
        system_comment(w, t, &reason)?;
        w.emit_as(
            "system",
            DomainEvent::TicketEscalated {
                ticket: t.id,
                key: t.key.clone(),
                reason: reason.clone(),
            },
        )?;
        w.emit_as(
            "system",
            DomainEvent::NeedsUser {
                ticket: Some(t.id),
                reason,
            },
        )?;
    }
    Ok(())
}
/// Dependency satisfaction of a ticket, read from the database.
pub(crate) fn deps_satisfied(c: &rusqlite::Connection, t: &Ticket) -> Result<bool> {
    for k in &t.depends_on {
        let d = tickets::get_by_key(c, t.project, k)?;
        if !dep_satisfied(&d) {
            return Ok(false);
        }
    }
    Ok(true)
}
/// Every Question/Permission that blocks the ticket is resolved.
pub(crate) fn all_blocking_resolved(c: &rusqlite::Connection, t: &Ticket) -> Result<bool> {
    Ok(comments::open_blocking(c, t.id)?.is_empty())
}
/// Project allowlist = manifest network allowlist + user-granted always-allow entries.
pub(crate) fn allowlist(app: &App, p: &projects::Project) -> Vec<String> {
    let mut v = p.allowlist.clone();
    if let Ok(m) = app.manifest(p) {
        v.extend(m.network.allowlist);
    }
    v
}
pub(crate) fn domain_allowed(domain: &str, list: &[String]) -> bool {
    let d = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    list.iter().any(|e| {
        let e = e.trim().to_ascii_lowercase();
        if let Some(suffix) = e.strip_prefix("*.") {
            d.ends_with(&format!(".{suffix}")) || d == suffix
        } else {
            d == e
        }
    })
}
/// Whether a permission request is inside the project allowlist (then the Lead may approve it for the run).
pub(crate) fn in_allowlist(req: &PermissionRequest, list: &[String]) -> bool {
    match req.kind {
        PermissionKind::Network => {
            !req.domains.is_empty() && req.domains.iter().all(|d| domain_allowed(d, list))
        }
        PermissionKind::Path => {
            !req.paths.is_empty()
                && req
                    .paths
                    .iter()
                    .all(|p| list.iter().any(|e| e == &format!("path:{p}")))
        }
        PermissionKind::Command => req
            .command
            .as_ref()
            .is_some_and(|c| list.iter().any(|e| e == &format!("cmd:{c}"))),
    }
}
/// Checks a new ticket against the plan, its dependencies and the open tickets' scopes, and inserts it (the caller's
/// transaction; `create_ticket` and `create_aggregate`).
pub(crate) fn insert_ticket(
    w: &mut WriteCtx<'_, '_>,
    pid: ProjectId,
    nt: NewTicket,
    default_template: String,
) -> Result<Ticket> {
    let plan = plans::current_plan(w.tx, pid)?.ok_or_else(|| {
        invalid("no approved plan: every ticket must reference a plan item (create_plan first)")
    })?;
    if !plan.body.items.iter().any(|i| i.id == nt.plan_item) {
        return Err(invalid(format!(
            "plan item {} is not in the approved plan v{}",
            nt.plan_item, plan.version
        )));
    }
    let parent = match &nt.parent {
        Some(k) => Some(ticket_by_key(w, pid, k)?),
        None => None,
    };
    let mut dep_ids = Vec::new();
    for k in &nt.depends_on {
        let d = ticket_by_key(w, pid, k)?;
        // A cancelled ticket is never done: a ticket waiting for it would never start.
        if d.status == TicketStatus::Cancelled {
            return Err(invalid(format!(
                "{} is cancelled and will never be done: drop it from depends_on (or depend on the ticket that replaces it)",
                d.key
            )));
        }
        dep_ids.push(d.id);
    }
    // Overlapping scope with an open ticket requires a dependency path between them (decision 18). A ticket that
    // declares no scope is read for the files it names (ADR-120): two tickets that both write `check.py` in parallel
    // duplicate the work and collide at the merge; the later one waits for the earlier and starts from its result.
    let own = effective_scope(&nt.scope_paths, &nt.title, &nt.acceptance);
    if !own.is_empty() {
        let graph = tickets::dep_graph(w.tx, pid)?;
        let ancestors = closure(&graph, &dep_ids);
        for other in tickets::list(w.tx, pid, &Default::default())? {
            if other.status.is_terminal() || ancestors.contains(&other.id) {
                continue;
            }
            let theirs = effective_scope(&other.scope_paths, &other.title, &other.acceptance);
            if !scopes_overlap(&own, &theirs) {
                continue;
            }
            let named = nt.scope_paths.is_empty() || other.scope_paths.is_empty();
            let shared: Vec<&String> = own
                .iter()
                .filter(|p| {
                    theirs
                        .iter()
                        .any(|q| scopes_overlap(std::slice::from_ref(*p), std::slice::from_ref(q)))
                })
                .collect();
            return Err(invalid(if named {
                format!(
                    "this ticket and open ticket {} both change {}: add {} to depends_on (it starts once {} is merged, from \
                     its work), or put the work in one ticket; if they change different files, declare scope_paths",
                    other.key,
                    shared.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
                    other.key,
                    other.key
                )
            } else {
                format!(
                    "scope_paths overlap with open ticket {} ({:?}); add it to depends_on or narrow the scope",
                    other.key, other.scope_paths
                )
            }));
        }
    }
    let key = projects::next_ticket_key(w.tx, pid)?;
    let id = TicketId::new();
    // A new ticket has no dependents yet, so it cannot close a cycle; checked anyway for safety.
    let graph = tickets::dep_graph(w.tx, pid)?;
    if let Some(cyc) = find_cycle(&graph, &id, &dep_ids) {
        return Err(invalid(format!(
            "depends_on would create a cycle ({} tickets)",
            cyc.len()
        )));
    }
    let t = tickets::insert(
        w.tx,
        &tickets::TicketInsert {
            id,
            key,
            project: pid,
            parent: parent.map(|p| p.id),
            kind: nt.kind,
            title: nt.title.trim().to_owned(),
            description: nt.description,
            acceptance: nt.acceptance,
            plan_item: nt.plan_item,
            priority: nt.priority,
            depends_on: dep_ids,
            required_role: nt.required_role,
            required_tags: nt.required_tags,
            scope_paths: nt.scope_paths,
            report_template: nt.report_template.unwrap_or(default_template),
            created_at: now_iso(),
        },
    )?;
    w.emit(DomainEvent::TicketCreated {
        ticket: t.id,
        key: t.key.clone(),
        title: t.title.clone(),
    })?;
    Ok(t)
}
pub(crate) fn closure(
    graph: &HashMap<TicketId, Vec<TicketId>>,
    start: &[TicketId],
) -> HashSet<TicketId> {
    let mut seen = HashSet::new();
    let mut stack: Vec<TicketId> = start.to_vec();
    while let Some(n) = stack.pop() {
        if seen.insert(n) {
            stack.extend(graph.get(&n).into_iter().flatten().copied());
        }
    }
    seen
}
/// In team mode the Lead takes no tickets: a plan item for the Lead's own role (when no worker has it) could never
/// be worked on. The Lead does such synthesis in its reviews, or gives the item to a worker role.
pub(crate) fn check_roles(
    w: &WriteCtx<'_, '_>,
    team: Option<&agents::Team>,
    items: &[agenium_core::plan::PlanItem],
) -> Result<()> {
    let Some(team) = team.filter(|t| !t.solo) else {
        return Ok(());
    };
    let Some(lead) = team.lead else {
        return Ok(());
    };
    let members = agents::agents_of_team(w.tx, team.id)?;
    let Some(lead_role) = members
        .iter()
        .find(|m| m.id == lead)
        .map(|m| m.role.as_str())
    else {
        return Ok(());
    };
    let workers: BTreeSet<&str> = members
        .iter()
        .filter(|m| m.id != lead)
        .map(|m| m.role.as_str())
        .collect();
    if workers.contains(lead_role) {
        return Ok(());
    }
    let theirs: Vec<&str> = items
        .iter()
        .filter(|i| i.role == lead_role)
        .map(|i| i.id.as_str())
        .collect();
    if theirs.is_empty() {
        return Ok(());
    }
    Err(invalid(format!(
        "plan items {} have role {lead_role}, the Lead's own role, and the Lead takes no tickets in team mode: give them to a worker role ({}), fold that work into your reviews, or propose_role when no worker fits",
        theirs.join(", "),
        workers.into_iter().collect::<Vec<_>>().join(", ")
    )))
}
/// Whether `new` only retypes `was` (case, spacing, punctuation): the same words, ids and criteria.
pub(crate) fn retyped(
    was: &agenium_core::plan::PlanItem,
    new: &agenium_core::plan::PlanItem,
) -> bool {
    let norm = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    norm(&was.title) == norm(&new.title)
        && was.acceptance.len() == new.acceptance.len()
        && was
            .acceptance
            .iter()
            .zip(&new.acceptance)
            .all(|(a, b)| a.id == b.id && norm(&a.text) == norm(&b.text))
}
/// What a ticket may change, for the overlap rule: its declared `scope_paths`, else the files it names in its title
/// and acceptance criteria (ADR-120).
pub(crate) fn effective_scope(
    declared: &[String],
    title: &str,
    acceptance: &[AcceptanceCriterion],
) -> Vec<String> {
    if !declared.is_empty() {
        return declared.to_vec();
    }
    let mut text = title.to_owned();
    for a in acceptance {
        text.push('\n');
        text.push_str(&a.text);
    }
    agenium_core::graph::paths_named(&text)
}
pub(crate) fn check_trace(spec: &SpecRow, items: &[agenium_core::plan::PlanItem]) -> Result<()> {
    let reqs: HashSet<&str> = spec
        .body
        .requirements
        .iter()
        .map(|r| r.id.as_str())
        .collect();
    for it in items {
        for t in &it.trace_to {
            if !reqs.contains(t.as_str()) {
                return Err(invalid(format!(
                    "plan item {} traces to {t}, which is not in spec v{}",
                    it.id, spec.version
                )));
            }
        }
    }
    Ok(())
}
/// Human-readable diff between plan versions and whether it changes scope (requirements covered or acceptance
/// criteria of existing items).
pub(crate) fn plan_diff(
    old: &[agenium_core::plan::PlanItem],
    new: &[agenium_core::plan::PlanItem],
) -> (String, bool) {
    let mut lines = Vec::new();
    let mut scope = false;
    let old_map: HashMap<&str, &agenium_core::plan::PlanItem> =
        old.iter().map(|i| (i.id.as_str(), i)).collect();
    let new_ids: HashSet<&str> = new.iter().map(|i| i.id.as_str()).collect();
    for it in new {
        match old_map.get(it.id.as_str()) {
            None => lines.push(format!("+ {} {}", it.id, it.title)),
            Some(o) => {
                if o.acceptance != it.acceptance {
                    lines.push(format!("~ {} acceptance changed", it.id));
                    scope = true;
                }
                if o.title != it.title
                    || o.depends_on != it.depends_on
                    || o.milestone != it.milestone
                {
                    lines.push(format!("~ {} {}", it.id, it.title));
                }
            }
        }
    }
    for o in old {
        if !new_ids.contains(o.id.as_str()) {
            lines.push(format!("- {} {}", o.id, o.title));
            scope = true;
        }
    }
    let covered = |v: &[agenium_core::plan::PlanItem]| {
        v.iter()
            .flat_map(|i| i.trace_to.iter().cloned())
            .collect::<HashSet<String>>()
    };
    if covered(old) != covered(new) {
        lines.push("~ requirement coverage changed".into());
        scope = true;
    }
    (lines.join("\n"), scope)
}
pub(crate) fn check_report_covers_acceptance(t: &Ticket, r: &WorkReport) -> Result<()> {
    let have: HashSet<&str> = r.acceptance.iter().map(|a| a.ac_id.as_str()).collect();
    let want: HashSet<&str> = t.acceptance.iter().map(|a| a.id.as_str()).collect();
    let missing: Vec<&&str> = want.difference(&have).collect();
    let unknown: Vec<&&str> = have.difference(&want).collect();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(invalid(format!(
            "the report must give one result per acceptance criterion of {}: missing {missing:?}, unknown {unknown:?}",
            t.key
        )));
    }
    Ok(())
}
/// After a blocking item is resolved: Pending -> InProgress (System) when nothing else blocks, agent -> Idle so the
/// scheduler starts a continuation run.
pub(crate) fn unblock_if_clear(w: &mut WriteCtx<'_, '_>, t: &mut Ticket) -> Result<()> {
    if t.status == TicketStatus::Pending && all_blocking_resolved(w.tx, t)? {
        transition(
            w,
            t,
            &Actor::System,
            TicketStatus::InProgress,
            &Guards {
                all_blocking_resolved: true,
                ..Default::default()
            },
        )?;
        if let Some(a) = t.assignee {
            set_agent_status(w, a, AgentStatus::Idle)?;
        }
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_permission(
    w: &mut WriteCtx<'_, '_>,
    t: &mut Ticket,
    c: &Comment,
    request: PermissionRequest,
    run: Option<RunId>,
    decision: PermissionDecision,
    by: &str,
    scope: &str,
    note: Option<String>,
) -> Result<()> {
    let body = CommentBody::Permission {
        request: request.clone(),
        run,
        outcome: Some(PermissionOutcome {
            decision,
            decided_by: by.to_owned(),
            scope: scope.to_owned(),
            note,
        }),
    };
    comments::set_body(w.tx, c.id, &body)?;
    comments::set_resolved(w.tx, c.id, true)?;
    w.emit(DomainEvent::CommentResolved {
        comment: c.id,
        ticket: t.id,
    })?;
    match decision {
        PermissionDecision::Approve => {
            runs::insert_grant(
                w.tx,
                &runs::Grant {
                    id: ulid::Ulid::new().to_string(),
                    project: t.project,
                    ticket: Some(t.id),
                    run: Some(run.unwrap_or_default_run()),
                    scope: scope.to_owned(),
                    granted_by: by.to_owned(),
                    request: serde_json::to_value(&request).unwrap_or_default(),
                },
            )?;
            w.emit(DomainEvent::PermissionGranted {
                ticket: t.id,
                request: c.id,
                scope: scope.to_owned(),
            })?;
        }
        PermissionDecision::Deny => {
            w.emit(DomainEvent::PermissionDenied {
                ticket: t.id,
                request: c.id,
            })?;
        }
    }
    unblock_if_clear(w, t)
}
/// Emits `PlanItemDone` when every ticket of the item is Closed (Cancelled ones do not count as done work but do
/// not block the item either).
/// What the reviewer does next, given the ticket after its review (and merge, if one ran).
pub(crate) fn review_next(t: &Ticket) -> String {
    match t.status {
        // Reopened clears `approved`: by the reviewer's feedback, or by a failed merge (System comment).
        TicketStatus::Reopened => {
            "the ticket went back to its worker (see the latest comment); stop here".into()
        }
        TicketStatus::Resolved if t.approved && (t.merged || !t.has_code) => format!(
            "{}call close_ticket for {} with a reason",
            if t.merged { "merged into main: " } else { "no code to merge: " },
            t.key
        ),
        TicketStatus::Resolved if t.approved => {
            "the merge did not complete yet; Agenium retries it and the ticket stays Resolved; stop here".into()
        }
        _ => format!("{} is {}", t.key, t.status),
    }
}
pub(crate) fn plan_item_done(w: &mut WriteCtx<'_, '_>, t: &Ticket) -> Result<()> {
    let f = tickets::TicketFilter {
        plan_item: Some(t.plan_item.clone()),
        ..Default::default()
    };
    let all = tickets::list(w.tx, t.project, &f)?;
    let done = all.iter().all(|x| x.status.is_terminal())
        && all.iter().any(|x| x.status == TicketStatus::Closed);
    if done {
        w.emit_as(
            "system",
            DomainEvent::PlanItemDone {
                project: t.project,
                item: t.plan_item.clone(),
            },
        )?;
    }
    Ok(())
}
/// A Solo agent (the Lead of a Solo team) acting on its own question or permission request: those are routed to the
/// user (`ask_question`, `request_permission`), so it may not decide them itself.
pub(crate) fn solo_self(team: Option<&agents::Team>, actor: &Actor, c: &Comment) -> bool {
    matches!(actor, Actor::Lead(a) if team.is_some_and(|t| t.solo) && c.author == Author::Agent(*a))
}
/// Whether the agent works as a Solo (its team is a Solo team).
pub(crate) fn solo_agent(
    c: &rusqlite::Connection,
    agent: AgentId,
) -> std::result::Result<bool, agenium_store::StoreError> {
    Ok(match agents::get_agent(c, agent)?.team {
        Some(t) => agents::get_team(c, t)?.solo,
        None => false,
    })
}

pub(crate) trait RunOrDefault {
    fn unwrap_or_default_run(self) -> RunId;
}
impl RunOrDefault for Option<RunId> {
    /// User-created requests (no run) still need a run id for the audit trigger: a fresh marker id.
    fn unwrap_or_default_run(self) -> RunId {
        self.unwrap_or_else(RunId::new)
    }
}
