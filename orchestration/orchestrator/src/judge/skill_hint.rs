//! Skill hint (ADR-82): before a work run, the skill that fits the ticket, so the worker loads one instead of reading
//! several to choose.

use agenium_core::Ticket;
use agenium_llm::systemone::{Judging, Question};

use super::MET;
use crate::app::App;

/// Description characters of each skill the judge reads: all of it (a skill's limit), since the "Use when" clause
/// that decides the pick comes last (ADR-141).
const DESCRIPTION: usize = 1024;

impl App {
    /// The judge's pick among `skills` (name, description) for `t`, as a hint for the worker. `None` when the judge is
    /// off/disabled, `skill_hint` is off, there are fewer than two skills, the pick is "none"/low-confidence/failed, or
    /// in shadow mode. A key/quota failure disables the judge.
    pub async fn skill_hint(&self, t: &Ticket, skills: &[(String, String)]) -> Option<String> {
        if !self.config().judge.skill_hint || skills.len() < 2 {
            return None;
        }
        self.judge().ok().flatten()?;
        let mut criteria: std::collections::BTreeMap<String, String> = skills
            .iter()
            .map(|(name, desc)| (name.clone(), desc.chars().take(DESCRIPTION).collect()))
            .collect();
        criteria.insert("none".into(), "no skill fits this ticket".into());
        let mut j = Judging::default().state_text(
            "ticket",
            &format!(
                "{}\n{}",
                t.title,
                t.description.chars().take(800).collect::<String>()
            ),
        );
        j.questions.insert(
            "skill".into(),
            Question::Choice {
                instructions: "Which skill best fits the work described in `ticket`?".into(),
                criteria,
            },
        );
        let key = t.key.to_string();
        let reply = self.ask(&j, "skill_hint", Some(&key)).await?;
        let a = reply.answers.get("skill");
        let pick = a.and_then(|a| a.choice.as_deref()).unwrap_or("none");
        let conf = a.and_then(|a| a.confidence);
        let pct = conf
            .map(|c| format!(" ({:.0}%)", c * 100.0))
            .unwrap_or_default();
        if pick == "none" || conf.is_some_and(|c| c < MET) {
            let outcome = format!(
                "no skill suggested (answer: {pick}{pct}) among {}",
                skills.len()
            );
            self.record("skill_hint", Some(&key), outcome, &reply);
            return None;
        }
        self.record(
            "skill_hint",
            Some(&key),
            format!("suggested `{pick}`{pct} among {} skills", skills.len()),
            &reply,
        );
        if !self.judge_acts() {
            return None;
        }
        Some(format!(
            "\nSuggested skill (TypeSafe Jev, a hint — load it only if it fits): `{pick}`. Call load_skill to read it before the work it covers.\n"
        ))
    }
}
