//! Whether a skill helps (ADR-152, criterion 12), measured for free from real work: the closed tickets whose runs
//! loaded it, how many of them were reopened, against the same agents' tickets without it (all other closed tickets
//! when those are too few). A skill is flagged only with enough tickets on both sides and a clearly higher reopen rate.

use std::collections::{BTreeMap, BTreeSet};

use agenium_store::core::skill_use::ClosedTicket;
use serde::Serialize;

/// Tickets needed on each side before a skill is judged.
pub const MIN_TICKETS: usize = 5;
/// A skill is worse when its tickets are reopened this much more often (as a share of tickets).
pub const WORSE_BY: f64 = 0.20;

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
}

fn rate(n: usize, of: usize) -> f64 {
    n as f64 / of.max(1) as f64
}

/// The effect of every skill loaded in a closed ticket.
pub fn effects(closed: &[ClosedTicket]) -> BTreeMap<String, Effect> {
    let skills: BTreeSet<&String> = closed.iter().flat_map(|t| &t.skills).collect();
    skills
        .into_iter()
        .map(|s| (s.clone(), effect_of(s, closed)))
        .collect()
}

pub fn effect_of(skill: &str, closed: &[ClosedTicket]) -> Effect {
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
    Effect {
        tickets: with.len(),
        reopened,
        others: base.len(),
        others_reopened,
        enough,
        worse: enough && rate(reopened, with.len()) - rate(others_reopened, base.len()) >= WORSE_BY,
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
    fn a_skill_is_flagged_only_with_enough_tickets_and_clearly_more_reopens() {
        let mut closed = Vec::new();
        // Agent a: 6 tickets with "slow" (4 reopened), 6 without (1 reopened).
        for i in 0..6 {
            closed.push(t(i, "a", i < 4, &["slow"]));
            closed.push(t(10 + i, "a", i < 1, &[]));
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
            (6, 4, 6, 1)
        );
        assert!(slow.enough && slow.worse, "{slow:?}");
        let fine = &e["fine"];
        assert_eq!(
            fine.others, 13,
            "too few of b's own: every other closed ticket"
        );
        assert!(fine.enough && !fine.worse, "{fine:?}");
        let rare = &e["rare"];
        assert!(!rare.enough && !rare.worse);
    }
}
