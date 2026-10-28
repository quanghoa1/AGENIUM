//! Skill quality (ADR-90) and contradictions between skills (ADR-92): advisory reads of a skill being imported or
//! changed. The deterministic checks (shape lint, one subject, opposite directives) run whatever the judge says.

use agenium_llm::systemone::Judging;

use super::MET;
use crate::app::App;

impl App {
    /// A calibrated read of a skill `body` against the methodology principles: one job, actionable steps, a proof that
    /// demands evidence, and no long embedded code that goes stale. One note per principle (e.g. "proof demands
    /// evidence: weak (41%)"). `None` when the judge is off/disabled, `skill_quality` is off, the call failed, or in
    /// shadow mode. A key/quota failure disables the judge.
    pub async fn skill_quality(&self, body: &str) -> Option<Vec<String>> {
        if !self.config().judge.skill_quality {
            return None;
        }
        let judge = self.judge().ok().flatten()?;
        let qs: [(&str, &str); 4] = [
            ("one job", "Does `skill` describe exactly one job, not several bundled together?"),
            ("actionable steps", "Are the steps in `skill` concrete and actionable rather than vague?"),
            ("proof demands evidence", "Does `skill` require concrete evidence (a proof) before the work may be called done?"),
            ("no stale code", "Is `skill` free of long embedded code blocks that would go out of date?"),
        ];
        let mut j = Judging::default().state_text("skill", body);
        for (id, q) in &qs {
            j = j.noul(id, q);
        }
        let reply = self.ask(&judge, &j, "skill_quality", None).await?;
        let notes: Vec<String> = qs
            .iter()
            .filter_map(|(id, _)| {
                let p = reply.answers.get(*id).and_then(|a| a.noul)?;
                let verdict = if p >= MET {
                    "yes"
                } else if p <= 1.0 - MET {
                    "no"
                } else {
                    "weak"
                };
                Some(format!("{id}: {verdict} ({:.0}%)", p * 100.0))
            })
            .collect();
        self.record("skill_quality", None, notes.join("; "), &reply);
        (!notes.is_empty() && self.judge_acts()).then_some(notes)
    }

    /// Whether a new or changed skill `text` contradicts the existing skills closest to it: one Noul per related skill
    /// — "do they give contradictory instructions for the same situation?". The names it confidently reads as
    /// contradicting (at or above `MET`). `None` when the judge is off/disabled, `skill_quality` is off, there is
    /// nothing related, the call failed, or in shadow mode. A key/quota failure disables the judge.
    pub async fn skill_contradiction(
        &self,
        text: &str,
        related: &[(String, String)],
    ) -> Option<Vec<String>> {
        if !self.config().judge.skill_quality || related.is_empty() {
            return None;
        }
        let judge = self.judge().ok().flatten()?;
        let cap = |s: &str| s.chars().take(6000).collect::<String>();
        let mut j = Judging::default().state_text("new_skill", &cap(text));
        for (i, (_, guidance)) in related.iter().enumerate() {
            let key = format!("existing_{i}");
            j = j.state_text(&key, &cap(guidance)).noul(
                &format!("c{i}"),
                &format!(
                    "Do `new_skill` and `{key}` give contradictory instructions for the same situation (one says to do \
                     something the other says not to do)?"
                ),
            );
        }
        let reply = self.ask(&judge, &j, "skill_contradiction", None).await?;
        let found: Vec<String> = related
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                reply
                    .answers
                    .get(&format!("c{i}"))
                    .and_then(|a| a.noul)
                    .is_some_and(|p| p >= MET)
            })
            .map(|(_, (name, _))| name.clone())
            .collect();
        let outcome = if found.is_empty() {
            format!("no contradiction with {} related skill(s)", related.len())
        } else {
            format!("contradicts {}", found.join(", "))
        };
        self.record("skill_contradiction", None, outcome, &reply);
        self.judge_acts().then_some(found)
    }
}
