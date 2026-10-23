//! Ticket state machine with guards (decision 3i, permission table of the architecture document).
//!
//! The orchestrator computes the guard facts from the database and calls [`check_transition`]; on success it
//! calls [`apply_transition`] to update the ticket. Both are pure so they can be property-tested.

use serde::{Deserialize, Serialize};

use crate::ids::AgentId;
use crate::ticket::{Ticket, TicketStatus, MAX_REOPEN_BEFORE_ESCALATE};

/// Who performs an action. A Solo agent acts as Lead for lead tools and as Worker for worker tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "agent")]
pub enum Actor {
    Lead(AgentId),
    Worker(AgentId),
    User,
    System,
}

impl Actor {
    /// Stable textual form stored in the event log / comments (`agent:<id>`, `user`, `system`).
    pub fn label(&self) -> String {
        match self {
            Actor::Lead(a) | Actor::Worker(a) => format!("agent:{a}"),
            Actor::User => "user".into(),
            Actor::System => "system".into(),
        }
    }

    pub fn agent(&self) -> Option<AgentId> {
        match self {
            Actor::Lead(a) | Actor::Worker(a) => Some(*a),
            _ => None,
        }
    }
}

/// Guard facts, computed by the orchestrator for the ticket at hand. Only the ones relevant to the requested
/// transition are read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Guards {
    /// New -> Assigned: the assignee is a worker of the ticket's team (Solo: the Solo agent).
    pub assignee_in_team: bool,
    /// The ticket references an existing plan item.
    pub has_plan_item: bool,
    /// Every `depends_on` is satisfied ([`dep_satisfied`]).
    pub deps_satisfied: bool,
    /// The assignee is Idle (scheduler start).
    pub assignee_idle: bool,
    /// InProgress -> Pending: a blocking Question/Permission is open.
    pub open_blocking: bool,
    /// Pending -> InProgress: every blocking item is resolved.
    pub all_blocking_resolved: bool,
    /// InProgress -> Resolved: the WorkReport is valid (template + every AC).
    pub valid_report: bool,
    /// Resolved -> Reopened by Lead/User: feedback for each unmet AC.
    pub per_ac_feedback: bool,
    /// Resolved -> Reopened by System: the merge failed (check log or conflict).
    pub merge_failed: bool,
    /// Resolved -> Closed: a reason was given.
    pub close_reason: bool,
    /// InProgress -> Assigned by System: no live run (crash recovery / run handed back).
    pub no_live_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[serde(tag = "code")]
pub enum RuleError {
    #[error("ticket is {status} (terminal); continue with a new ticket linked follow_up_of")]
    Terminal { status: TicketStatus },
    #[error("transition {from} -> {to} is not allowed for {actor}")]
    Forbidden {
        from: TicketStatus,
        to: TicketStatus,
        actor: String,
    },
    #[error("guard failed for {from} -> {to}: {reason}")]
    Guard {
        from: TicketStatus,
        to: TicketStatus,
        reason: String,
    },
}

fn guard(ok: bool, t: &Ticket, to: TicketStatus, reason: &str) -> Result<(), RuleError> {
    if ok {
        Ok(())
    } else {
        Err(RuleError::Guard {
            from: t.status,
            to,
            reason: reason.into(),
        })
    }
}

/// Checks whether `actor` may move `t` to `to` given the guard facts.
pub fn check_transition(
    t: &Ticket,
    g: &Guards,
    actor: &Actor,
    to: TicketStatus,
) -> Result<(), RuleError> {
    use TicketStatus::*;
    let from = t.status;
    if from.is_terminal() {
        return Err(RuleError::Terminal { status: from });
    }
    let is_assignee = |a: &AgentId| t.assignee == Some(*a);
    match (from, to, actor) {
        (New, Assigned, Actor::Lead(_) | Actor::User) => {
            guard(
                t.assignee.is_some() && g.assignee_in_team,
                t,
                to,
                "assignee must be a worker of the team",
            )?;
            guard(
                g.has_plan_item,
                t,
                to,
                "ticket must reference an existing plan item",
            )
        }
        (Assigned | Reopened, InProgress, Actor::System) => {
            guard(t.assignee.is_some(), t, to, "ticket has no assignee")?;
            guard(g.deps_satisfied, t, to, "dependencies are not satisfied")?;
            guard(g.assignee_idle, t, to, "assignee is not Idle")
        }
        (InProgress, Assigned, Actor::System) => {
            guard(g.no_live_run, t, to, "a run is still alive")
        }
        (InProgress, Pending, Actor::Worker(a)) if is_assignee(a) => guard(
            g.open_blocking,
            t,
            to,
            "no open blocking question or permission request",
        ),
        (Pending, InProgress, Actor::System | Actor::User) => guard(
            g.all_blocking_resolved,
            t,
            to,
            "a blocking item is still open",
        ),
        (InProgress, Resolved, Actor::Worker(a)) if is_assignee(a) => {
            guard(g.valid_report, t, to, "work report is not valid")
        }
        (Resolved, Reopened, Actor::Lead(_) | Actor::User) => guard(
            g.per_ac_feedback,
            t,
            to,
            "reopen needs feedback for each unmet acceptance criterion",
        ),
        (Resolved, Reopened, Actor::System) => guard(
            g.merge_failed,
            t,
            to,
            "system reopen only after a failed merge",
        ),
        (Resolved, Closed, Actor::Lead(_) | Actor::User) => {
            guard(g.close_reason, t, to, "close needs a reason")?;
            guard(
                !t.has_code || t.merged,
                t,
                to,
                "code ticket is not merged yet (merge does not close; close after merge)",
            )
        }
        (_, Cancelled, Actor::Lead(_) | Actor::User) => Ok(()),
        _ => Err(RuleError::Forbidden {
            from,
            to,
            actor: actor_name(actor, t),
        }),
    }
}

fn actor_name(actor: &Actor, t: &Ticket) -> String {
    match actor {
        Actor::Lead(_) => "Lead".into(),
        Actor::Worker(a) if t.assignee == Some(*a) => "Worker (assignee)".into(),
        Actor::Worker(_) => "Worker (not the assignee)".into(),
        Actor::User => "User".into(),
        Actor::System => "System".into(),
    }
}

/// Side effects of a transition beyond the status change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransitionEffects {
    /// The ticket crossed the reopen threshold and is now flagged for the user (flag, not a status).
    pub escalated_now: bool,
}

/// Applies an already checked transition.
pub fn apply_transition(t: &mut Ticket, to: TicketStatus) -> TransitionEffects {
    let mut fx = TransitionEffects::default();
    if to == TicketStatus::Reopened {
        t.reopen_count += 1;
        // A reopened ticket must be reviewed (and merged) again.
        t.approved = false;
        t.merged = false;
        if t.reopen_count > MAX_REOPEN_BEFORE_ESCALATE && !t.escalated {
            t.escalated = true;
            fx.escalated_now = true;
        }
    }
    t.status = to;
    fx
}

/// A dependency is satisfied when the predecessor is Closed, or Resolved + approved (+ merged for code tickets).
/// No need to wait for `close_ticket`; there is no Merging state (decision 36).
pub fn dep_satisfied(d: &Ticket) -> bool {
    match d.status {
        TicketStatus::Closed => true,
        TicketStatus::Resolved => d.approved && (!d.has_code || d.merged),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ProjectId, TicketId, TicketKey};
    use crate::ticket::{AcceptanceCriterion, TicketKind, UsageSummary};
    use proptest::prelude::*;

    fn ticket(status: TicketStatus, assignee: Option<AgentId>) -> Ticket {
        Ticket {
            id: TicketId::new(),
            key: TicketKey::new("AGN", 1).unwrap(),
            project: ProjectId::new(),
            parent: None,
            kind: TicketKind::Task,
            title: "t".into(),
            description: String::new(),
            acceptance: vec![AcceptanceCriterion {
                id: "AC-1".into(),
                text: "a".into(),
            }],
            status,
            assignee,
            plan_item: "PI-1".into(),
            priority: 2,
            depends_on: vec![],
            reopen_count: 0,
            escalated: false,
            required_role: None,
            required_tags: vec![],
            scope_paths: vec![],
            report_template: "coding@1".into(),
            approved: false,
            merged: false,
            has_code: false,
            close_reason: None,
            usage: UsageSummary::default(),
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    const ALL_GUARDS: Guards = Guards {
        assignee_in_team: true,
        has_plan_item: true,
        deps_satisfied: true,
        assignee_idle: true,
        open_blocking: true,
        all_blocking_resolved: true,
        valid_report: true,
        per_ac_feedback: true,
        merge_failed: true,
        close_reason: true,
        no_live_run: true,
    };

    /// The permission table of the architecture document, row by row (M3-2 table-driven test seed).
    #[test]
    fn permission_table() {
        use TicketStatus::*;
        let w = AgentId::new();
        let other = AgentId::new();
        let lead = Actor::Lead(AgentId::new());
        let rows: &[(TicketStatus, TicketStatus, Actor, bool)] = &[
            (New, Assigned, lead, true),
            (New, Assigned, Actor::User, true),
            (New, Assigned, Actor::Worker(w), false),
            (New, Assigned, Actor::System, false),
            (Assigned, InProgress, Actor::System, true),
            (Reopened, InProgress, Actor::System, true),
            (Assigned, InProgress, lead, false),
            (Assigned, InProgress, Actor::Worker(w), false),
            (Assigned, InProgress, Actor::User, false),
            (InProgress, Pending, Actor::Worker(w), true),
            (InProgress, Pending, Actor::Worker(other), false),
            (InProgress, Pending, lead, false),
            (Pending, InProgress, Actor::System, true),
            (Pending, InProgress, Actor::User, true),
            (Pending, InProgress, Actor::Worker(w), false),
            (InProgress, Resolved, Actor::Worker(w), true),
            (InProgress, Resolved, Actor::Worker(other), false),
            (InProgress, Resolved, lead, false),
            (Resolved, Reopened, lead, true),
            (Resolved, Reopened, Actor::User, true),
            (Resolved, Reopened, Actor::System, true),
            (Resolved, Reopened, Actor::Worker(w), false),
            (Resolved, Closed, lead, true),
            (Resolved, Closed, Actor::User, true),
            (Resolved, Closed, Actor::System, false),
            (Resolved, Closed, Actor::Worker(w), false),
            (InProgress, Closed, lead, false),
            (New, Cancelled, lead, true),
            (Pending, Cancelled, Actor::User, true),
            (Resolved, Cancelled, lead, true),
            (InProgress, Cancelled, Actor::Worker(w), false),
            (InProgress, Assigned, Actor::System, true),
            (InProgress, Assigned, lead, false),
        ];
        for &(from, to, actor, ok) in rows {
            let t = ticket(from, Some(w));
            assert_eq!(
                check_transition(&t, &ALL_GUARDS, &actor, to).is_ok(),
                ok,
                "{from} -> {to} by {actor:?}"
            );
        }
    }

    #[test]
    fn closed_requires_merge_for_code_tickets() {
        let mut t = ticket(TicketStatus::Resolved, Some(AgentId::new()));
        t.has_code = true;
        let lead = Actor::Lead(AgentId::new());
        let e = check_transition(&t, &ALL_GUARDS, &lead, TicketStatus::Closed).unwrap_err();
        assert!(matches!(e, RuleError::Guard { .. }), "{e}");
        t.merged = true;
        check_transition(&t, &ALL_GUARDS, &lead, TicketStatus::Closed).unwrap();
        // A ticket without code closes directly after review.
        let t2 = ticket(TicketStatus::Resolved, None);
        check_transition(&t2, &ALL_GUARDS, &lead, TicketStatus::Closed).unwrap();
    }

    #[test]
    fn reopen_resets_flags_and_escalates_after_three() {
        let mut t = ticket(TicketStatus::Resolved, Some(AgentId::new()));
        for i in 1..=4 {
            t.status = TicketStatus::Resolved;
            t.approved = true;
            t.merged = true;
            let fx = apply_transition(&mut t, TicketStatus::Reopened);
            assert_eq!(t.reopen_count, i);
            assert!(!t.approved && !t.merged);
            assert_eq!(fx.escalated_now, i == 4);
            assert_eq!(t.escalated, i == 4);
        }
    }

    #[test]
    fn guard_failures_are_reported() {
        let t = ticket(TicketStatus::Assigned, Some(AgentId::new()));
        let g = Guards {
            deps_satisfied: false,
            ..ALL_GUARDS
        };
        let e = check_transition(&t, &g, &Actor::System, TicketStatus::InProgress).unwrap_err();
        assert_eq!(
            e.to_string(),
            "guard failed for Assigned -> InProgress: dependencies are not satisfied"
        );
    }

    fn arb_status() -> impl Strategy<Value = TicketStatus> {
        prop::sample::select(TicketStatus::ALL.to_vec())
    }

    fn arb_guards() -> impl Strategy<Value = Guards> {
        prop::array::uniform11(any::<bool>()).prop_map(|b| Guards {
            assignee_in_team: b[0],
            has_plan_item: b[1],
            deps_satisfied: b[2],
            assignee_idle: b[3],
            open_blocking: b[4],
            all_blocking_resolved: b[5],
            valid_report: b[6],
            per_ac_feedback: b[7],
            merge_failed: b[8],
            close_reason: b[9],
            no_live_run: b[10],
        })
    }

    /// Allowed edges of the lifecycle graph (decision 3i) + Cancelled from any non-terminal state.
    fn edge_allowed(from: TicketStatus, to: TicketStatus) -> bool {
        use TicketStatus::*;
        matches!(
            (from, to),
            (New, Assigned)
                | (Assigned, InProgress)
                | (Reopened, InProgress)
                | (InProgress, Assigned)
                | (InProgress, Pending)
                | (Pending, InProgress)
                | (InProgress, Resolved)
                | (Resolved, Reopened)
                | (Resolved, Closed)
        ) || (to == Cancelled && !from.is_terminal())
    }

    proptest! {
        #[test]
        fn no_illegal_transition(from in arb_status(), to in arb_status(), g in arb_guards(),
                                 actor_kind in 0u8..5, assignee_is_actor in any::<bool>(), has_code in any::<bool>(),
                                 merged in any::<bool>()) {
            let a = AgentId::new();
            let mut t = ticket(from, Some(a));
            t.has_code = has_code;
            t.merged = merged;
            let actor = match actor_kind {
                0 => Actor::Lead(AgentId::new()),
                1 => Actor::Worker(if assignee_is_actor { a } else { AgentId::new() }),
                2 => Actor::User,
                _ => Actor::System,
            };
            if check_transition(&t, &g, &actor, to).is_ok() {
                prop_assert!(edge_allowed(from, to), "{from} -> {to} accepted for {actor:?}");
                if to == TicketStatus::Closed {
                    prop_assert!(!t.has_code || t.merged, "closed without merge");
                    prop_assert!(matches!(actor, Actor::Lead(_) | Actor::User), "only Lead/User close");
                }
                if let Actor::Worker(w) = actor {
                    prop_assert_eq!(Some(w), t.assignee, "workers only act on their own ticket");
                }
            }
        }

        #[test]
        fn terminal_states_stay_terminal(to in arb_status(), g in arb_guards(), actor_kind in 0u8..4,
                                         closed in any::<bool>()) {
            let a = AgentId::new();
            let t = ticket(if closed { TicketStatus::Closed } else { TicketStatus::Cancelled }, Some(a));
            let actor = [Actor::Lead(a), Actor::Worker(a), Actor::User, Actor::System][actor_kind as usize];
            let terminal = matches!(check_transition(&t, &g, &actor, to), Err(RuleError::Terminal { .. }));
            prop_assert!(terminal);
        }

        #[test]
        fn dep_satisfied_matches_rule(status in arb_status(), approved in any::<bool>(), merged in any::<bool>(),
                                      has_code in any::<bool>()) {
            let mut d = ticket(status, None);
            d.approved = approved;
            d.merged = merged;
            d.has_code = has_code;
            let expected = status == TicketStatus::Closed
                || (status == TicketStatus::Resolved && approved && (!has_code || merged));
            prop_assert_eq!(dep_satisfied(&d), expected);
        }
    }
}
