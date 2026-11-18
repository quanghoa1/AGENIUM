//! Whether a skill helps (ADR-152, criterion 12), measured for free from real work: the closed tickets whose runs
//! loaded it, how many of them were reopened, against the same agents' tickets without it (all other closed tickets
//! when those are too few). A skill is flagged only with enough tickets on both sides and a reopen rate higher by
//! more than chance explains: the 95% interval of the difference (Newcombe's, from Wilson intervals) lies above zero,
//! and the difference is at least `WORSE_BY`. One reopened ticket of five against none of five flagged a skill before
//! (NVIDIA SkillEvaluator's review, 2026-11-18). The comparison is not controlled: a skill is loaded on the tickets it
//! fits, which may be reopened more anyway; so it is a reason to look at the skill, never a verdict.

use std::collections::{BTreeMap, BTreeSet};

use agenium_store::core::skill_use::ClosedTicket;
use serde::Serialize;

/// Tickets needed on each side before a skill is judged.
pub const MIN_TICKETS: usize = 5;
/// A skill is worse when its tickets are reopened this much more often (as a share of tickets).
pub(crate) const WORSE_BY: f64 = 0.20;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Effect {
    /// Closed tickets whose runs loaded the skill, and how many of them were reopened.
    pub tickets: usize,
    pub reopened: usize,
    /// The tickets compared against, and how many of them were reopened.
    pub others: usize,
    pub others_reopened: usize,
    /// Enough tickets on both sides to judge.
    pub enough: bool,
    /// Clearly more reopens with the skill than without.
    pub worse: bool,
    /// The 95% interval of the difference of the reopen rates (with the skill minus without), in points (-100..100).
    pub diff_low: i32,
    pub diff_high: i32,
}

fn rate(n: usize, of: usize) -> f64 {
    n as f64 / of.max(1) as f64
}

/// 95% Wilson interval of `k` successes in `n`.
fn wilson(k: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    const Z: f64 = 1.96;
    let (n, p) = (n as f64, k as f64 / n as f64);
    let d = 1.0 + Z * Z / n;
    let center = (p + Z * Z / (2.0 * n)) / d;
    let half = Z * (p * (1.0 - p) / n + Z * Z / (4.0 * n * n)).sqrt() / d;
    ((center - half).max(0.0), (center + half).min(1.0))
}

/// 95% interval of `k1/n1 - k2/n2` (Newcombe's hybrid score method).
fn diff_interval(k1: usize, n1: usize, k2: usize, n2: usize) -> (f64, f64) {
    let (p1, p2) = (rate(k1, n1), rate(k2, n2));
    let ((l1, u1), (l2, u2)) = (wilson(k1, n1), wilson(k2, n2));
    let d = p1 - p2;
    (
        d - ((p1 - l1).powi(2) + (u2 - p2).powi(2)).sqrt(),
        d + ((u1 - p1).powi(2) + (p2 - l2).powi(2)).sqrt(),
    )
}

/// Closed tickets of agents that have a skill switched on before an unused skill is said so.
pub const UNUSED_AFTER: usize = 10;

/// The closed tickets of `on_for` (the agents that have `skill` switched on) when no closed ticket ever loaded it:
/// a skill its agents never pick is one whose description does not say when it applies (or one they do not need).
/// `None` when it was loaded somewhere, or no such agent closed a ticket.
pub fn never_loaded(skill: &str, on_for: &[String], closed: &[ClosedTicket]) -> Option<usize> {
    if closed.iter().any(|t| t.skills.contains(skill)) {
        return None;
    }
    let n = closed
        .iter()
        .filter(|t| t.assignee.as_ref().is_some_and(|a| on_for.contains(a)))
        .count();
    (n > 0).then_some(n)
}

/// The effect of every skill loaded in a closed ticket.
pub fn effects(closed: &[ClosedTicket]) -> BTreeMap<String, Effect> {
    let skills: BTreeSet<&String> = closed.iter().flat_map(|t| &t.skills).collect();
    skills
        .into_iter()
        .map(|s| (s.clone(), effect_of(s, closed)))
        .collect()
}

pub(crate) fn effect_of(skill: &str, closed: &[ClosedTicket]) -> Effect {
    let (with, without): (Vec<&ClosedTicket>, Vec<&ClosedTicket>) =
        closed.iter().partition(|t| t.skills.contains(skill));
    let agents: BTreeSet<&Option<String>> = with.iter().map(|t| &t.assignee).collect();
    let same: Vec<&&ClosedTicket> = without
        .iter()
        .filter(|t| agents.contains(&t.assignee))
        .collect();
    let base: Vec<&ClosedTicket> = if same.len() >= MIN_TICKETS {
        same.into_iter().copied().collect()
    } else {
        without
    };
    let reopened = with.iter().filter(|t| t.reopened).count();
    let others_reopened = base.iter().filter(|t| t.reopened).count();
    let enough = with.len() >= MIN_TICKETS && base.len() >= MIN_TICKETS;
    let (low, high) = diff_interval(reopened, with.len(), others_reopened, base.len());
    let worse = enough
        && low > 0.0
        && rate(reopened, with.len()) - rate(others_reopened, base.len()) >= WORSE_BY;
    Effect {
        tickets: with.len(),
        reopened,
        others: base.len(),
        others_reopened,
        enough,
        worse,
        diff_low: (low * 100.0).round() as i32,
        diff_high: (high * 100.0).round() as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(n: usize, agent: &str, reopened: bool, skills: &[&str]) -> ClosedTicket {
        ClosedTicket {
            ticket: format!("t{n}"),
            assignee: Some(agent.into()),
            reopened,
            skills: skills.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn the_intervals_are_wilsons_and_newcombes() {
        let (l, u) = wilson(4, 6);
        assert!(
            (l - 0.300).abs() < 0.005 && (u - 0.903).abs() < 0.005,
            "{l} {u}"
        );
        // 1 reopened of 5 against none of 5: chance explains it.
        let (low, _) = diff_interval(1, 5, 0, 5);
        assert!(low < 0.0, "{low}");
        let (low, _) = diff_interval(8, 10, 1, 10);
        assert!(low > 0.0, "{low}");
    }

    #[test]
    fn a_skill_is_flagged_only_with_enough_tickets_and_clearly_more_reopens() {
        let mut closed = Vec::new();
        // Agent a: 10 tickets with "slow" (8 reopened), 10 without (1 reopened).
        for i in 0..10 {
            closed.push(t(i, "a", i < 8, &["slow"]));
            closed.push(t(100 + i, "a", i < 1, &[]));
        }
        // Agent d: 5 tickets with "noisy" (1 reopened), 5 without (none): not flagged, chance explains it.
        for i in 0..5 {
            closed.push(t(200 + i, "d", i < 1, &["noisy"]));
            closed.push(t(300 + i, "d", false, &[]));
        }
        // Agent b: 6 tickets with "fine" (1 reopened); its other tickets are too few, so all others count.
        for i in 0..6 {
            closed.push(t(20 + i, "b", i < 1, &["fine"]));
        }
        closed.push(t(30, "c", false, &["rare"]));
        let e = effects(&closed);
        let slow = &e["slow"];
        assert_eq!(
            (
                slow.tickets,
                slow.reopened,
                slow.others,
                slow.others_reopened
            ),
            (10, 8, 10, 1)
        );
        assert!(slow.enough && slow.worse && slow.diff_low > 0, "{slow:?}");
        let noisy = &e["noisy"];
        assert!(
            noisy.enough && !noisy.worse && noisy.diff_low < 0,
            "{noisy:?}"
        );
        let fine = &e["fine"];
        assert_eq!(
            fine.others, 31,
            "too few of b's own: every other closed ticket"
        );
        assert!(fine.enough && !fine.worse, "{fine:?}");
        let rare = &e["rare"];
        assert!(!rare.enough && !rare.worse);
        // Switched on for agent a, which closed 20 tickets, and never loaded: said; loaded somewhere: not.
        assert_eq!(never_loaded("unpicked", &["a".into()], &closed), Some(20));
        assert_eq!(never_loaded("slow", &["a".into()], &closed), None);
        assert_eq!(never_loaded("unpicked", &["z".into()], &closed), None);
    }
}
