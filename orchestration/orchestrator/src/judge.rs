//! The optional TypeSafe "Jev" judge (ADR-75, ADR-81): a cheap, calibrated second opinion, off until `[judge]` is
//! set. Agenium never uses it as an agent's model, and it runs in two modes — with Jev and without. Without Jev (no
//! config, or after a failure) every judge call simply returns nothing and the work proceeds on the agent's own
//! judgment, so Agenium always works. A call that fails on the key or the quota turns the judge off and asks the
//! user to re-enter the key and re-enable it; a run error does not.
//!
//! Its wired use is an independent evidence read before a review run (`review_assist`): one batched request, a Noul
//! per acceptance criterion, a token-light first opinion for the Lead's expensive model.

use agenium_core::{DomainEvent, Ticket};
use agenium_llm::systemone::{Judge, Judging, Question};
use agenium_llm::LlmError;
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::{OrchError, Result};

/// A Noul at or above this reads as "met", at or below `1 - this` as "not met", between as "unsure" (decision 35).
const MET: f64 = 0.7;

/// Whether the judge is usable now and, if not, why (persisted in `<root>/judge-health.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JudgeHealth {
    /// `true` when a key/quota failure turned the judge off until the user re-enables it.
    pub disabled: bool,
    pub reason: Option<String>,
    pub at: Option<String>,
}

/// A key/quota failure disables the judge (the user must act); a one-off run error does not.
fn is_credential_or_quota(e: &LlmError) -> bool {
    match e {
        // systemone maps 401/403/400/422 to Config and 429 to a "rate limited" Network error.
        LlmError::Config(m) => {
            let m = m.to_lowercase();
            m.contains("key") || m.contains("subscription") || m.contains("rejected")
        }
        LlmError::Network(m) => m.to_lowercase().contains("rate limited"),
        _ => false,
    }
}

impl App {
    pub fn judge_health(&self) -> JudgeHealth {
        std::fs::read_to_string(self.layout().judge_health())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn set_judge_health(&self, h: &JudgeHealth) -> Result<()> {
        let path = self.layout().judge_health();
        std::fs::write(
            &path,
            serde_json::to_string_pretty(h).map_err(|e| OrchError::Invalid(e.to_string()))?,
        )
        .map_err(|e| OrchError::Invalid(format!("{}: {e}", path.display())))
    }

    /// Turns the judge off after a key/quota failure and tells the user to re-enter the key and re-enable it.
    fn disable_judge(&self, reason: String) -> Result<()> {
        if self.judge_health().disabled {
            return Ok(());
        }
        self.set_judge_health(&JudgeHealth {
            disabled: true,
            reason: Some(reason.clone()),
            at: Some(agenium_core::now_iso()),
        })?;
        self.store().commit(None, "system", |w| -> Result<()> {
            w.emit(DomainEvent::NeedsUser {
                ticket: None,
                reason: format!("the TypeSafe judge was turned off ({reason}); re-enter its key and re-enable it, or leave it off"),
            })?;
            Ok(())
        })?;
        Ok(())
    }

    /// The configured judge, or `None` when `[judge]` is off or a failure disabled it. `Err` only when it is on,
    /// enabled, but misconfigured (no key).
    pub fn judge(&self) -> Result<Option<Judge>> {
        let j = self.config().judge;
        if !j.enabled() || self.judge_health().disabled {
            return Ok(None);
        }
        let key = if j.api_key_ref.is_empty() {
            String::new()
        } else {
            self.secret(&j.api_key_ref)?
                .ok_or_else(|| {
                    OrchError::NotFound(format!("judge key {:?} is not set", j.api_key_ref))
                })?
                .expose_str()
                .map_err(|e| OrchError::Invalid(e.to_string()))?
                .to_owned()
        };
        Judge::new(&j.base_url, &key, &j.model)
            .map(Some)
            .map_err(|e| OrchError::Invalid(e.to_string()))
    }

    /// Re-enables the judge and checks it with a trivial request; clears the disabled state on success. For the user
    /// after re-entering the key.
    pub async fn enable_judge(&self) -> Result<()> {
        self.set_judge_health(&JudgeHealth::default())?;
        let Some(judge) = self.judge()? else {
            return Err(OrchError::Invalid(
                "the judge is not configured: set [judge] base_url, model and the key first".into(),
            ));
        };
        let probe = Judging::default()
            .state_text("x", "ok")
            .noul("q", "Does `x` say ok?");
        match judge.ask(&probe).await {
            Ok(_) => Ok(()),
            Err(e) => {
                self.set_judge_health(&JudgeHealth {
                    disabled: true,
                    reason: Some(e.to_string()),
                    at: Some(agenium_core::now_iso()),
                })?;
                Err(OrchError::Invalid(format!("the judge still fails: {e}")))
            }
        }
    }

    /// Before a work run, lets the judge pick the skill that best fits the ticket from `skills` (name, description),
    /// so the worker loads the right one without reading them all (ADR-82). `None` when the judge is off/disabled,
    /// `skill_hint` is off, there are fewer than two skills, or the pick is "none"/low-confidence/failed. A
    /// key/quota failure disables the judge.
    pub async fn skill_hint(&self, t: &Ticket, skills: &[(String, String)]) -> Option<String> {
        if !self.config().judge.skill_hint || skills.len() < 2 {
            return None;
        }
        let judge = match self.judge() {
            Ok(Some(j)) => j,
            _ => return None,
        };
        let mut criteria: std::collections::BTreeMap<String, String> = skills
            .iter()
            .map(|(name, desc)| (name.clone(), desc.chars().take(200).collect()))
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
        let answers = match judge.ask(&j).await {
            Ok(a) => a,
            Err(e) => {
                if is_credential_or_quota(&e) {
                    let _ = self.disable_judge(e.to_string());
                }
                return None;
            }
        };
        let a = answers.get("skill")?;
        let pick = a.choice.as_deref()?;
        if pick == "none" || a.confidence.is_some_and(|c| c < MET) {
            return None;
        }
        Some(format!(
            "\nSuggested skill (TypeSafe Jev, a hint — load it only if it fits): `{pick}`. Call load_skill to read it before the work it covers.\n"
        ))
    }

    /// After a ticket closes, a cheap read of whether the `episode` holds anything worth remembering, so the costly
    /// extraction model can be skipped on routine work (ADR-85). `Some(false)` only when the judge is confident
    /// nothing is worth keeping; `Some(true)` when it is; `None` when the judge is off/disabled, `memory_gate` is off,
    /// the verdict is unsure, or the call failed — the caller then extracts as usual. A key/quota failure disables it.
    pub async fn worth_remembering(&self, episode: &str) -> Option<bool> {
        if !self.config().judge.memory_gate {
            return None;
        }
        let judge = match self.judge() {
            Ok(Some(j)) => j,
            _ => return None,
        };
        let j = Judging::default().state_text("episode", episode).noul(
            "worth",
            "Does `episode` contain anything worth remembering for future tickets — a durable fact, preference, \
             convention, path/command, or (if it names a signal like a reopen or correction) a lesson — as opposed to \
             only routine work with nothing new?",
        );
        let answers = match judge.ask(&j).await {
            Ok(a) => a,
            Err(e) => {
                if is_credential_or_quota(&e) {
                    let _ = self.disable_judge(e.to_string());
                }
                return None;
            }
        };
        let p = answers.get("worth").and_then(|a| a.noul)?;
        if p >= MET {
            Some(true)
        } else if p <= 1.0 - MET {
            Some(false)
        } else {
            None
        }
    }

    /// A calibrated read of a skill `body` against the methodology principles (ADR-90): one job, actionable steps, a
    /// proof that demands evidence, and no long embedded code that goes stale. Returns a human-readable note per
    /// principle (e.g. "proof demands evidence: weak (41%)"). `None` when the judge is off/disabled, `skill_quality`
    /// is off, or the call failed — the deterministic shape check (`lint_skill`) still runs regardless. A key/quota
    /// failure disables the judge.
    pub async fn skill_quality(&self, body: &str) -> Option<Vec<String>> {
        if !self.config().judge.skill_quality {
            return None;
        }
        let judge = match self.judge() {
            Ok(Some(j)) => j,
            _ => return None,
        };
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
        let answers = match judge.ask(&j).await {
            Ok(a) => a,
            Err(e) => {
                if is_credential_or_quota(&e) {
                    let _ = self.disable_judge(e.to_string());
                }
                return None;
            }
        };
        let notes: Vec<String> = qs
            .iter()
            .filter_map(|(id, _)| {
                let p = answers.get(*id).and_then(|a| a.noul)?;
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
        (!notes.is_empty()).then_some(notes)
    }

    /// The judge's read of whether a new or changed skill `text` contradicts the existing skills closest to it
    /// (ADR-92): one Noul per related skill — "do they give contradictory instructions for the same situation?".
    /// Returns the names it confidently reads as contradicting (at or above `MET`). `None` when the judge is
    /// off/disabled, `skill_quality` is off, there is nothing related, or the call failed — the deterministic checks
    /// (one subject, explicit opposite directives) still apply. A key/quota failure disables the judge.
    pub async fn skill_contradiction(
        &self,
        text: &str,
        related: &[(String, String)],
    ) -> Option<Vec<String>> {
        if !self.config().judge.skill_quality || related.is_empty() {
            return None;
        }
        let judge = match self.judge() {
            Ok(Some(j)) => j,
            _ => return None,
        };
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
        let answers = match judge.ask(&j).await {
            Ok(a) => a,
            Err(e) => {
                if is_credential_or_quota(&e) {
                    let _ = self.disable_judge(e.to_string());
                }
                return None;
            }
        };
        Some(
            related
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    answers
                        .get(&format!("c{i}"))
                        .and_then(|a| a.noul)
                        .is_some_and(|p| p >= MET)
                })
                .map(|(_, (name, _))| name.clone())
                .collect(),
        )
    }

    /// An independent first read of `report` against `t`'s acceptance criteria, for the review task (ADR-75).
    /// `None` when the judge is off/disabled, `review_assist` is off, there are no criteria, or the call failed —
    /// the review then proceeds on the Lead's own read. A key/quota failure disables the judge.
    pub async fn review_evidence(&self, t: &Ticket, report: &str) -> Option<String> {
        if !self.config().judge.review_assist || t.acceptance.is_empty() {
            return None;
        }
        let judge = match self.judge() {
            Ok(Some(j)) => j,
            Ok(None) => return None,
            Err(e) => {
                tracing::warn!(error = %e, "judge not available");
                return None;
            }
        };
        let mut j = Judging::default().state_text("report", report);
        for ac in &t.acceptance {
            j = j.noul(
                &ac.id,
                &format!("Does the work described in `report` provide evidence that this criterion is met: {}", ac.text),
            );
        }
        let answers = match judge.ask(&j).await {
            Ok(a) => a,
            Err(e) => {
                if is_credential_or_quota(&e) {
                    let _ = self.disable_judge(e.to_string());
                } else {
                    tracing::warn!(error = %e, "judge evidence read failed");
                }
                return None;
            }
        };
        let mut lines = Vec::new();
        for ac in &t.acceptance {
            if let Some(p) = answers.get(&ac.id).and_then(|a| a.noul) {
                let verdict = if p >= MET {
                    "evidence found"
                } else if p <= 1.0 - MET {
                    "no evidence"
                } else {
                    "unclear"
                };
                lines.push(format!("- {}: {verdict} ({:.0}%)", ac.id, p * 100.0));
            }
        }
        if lines.is_empty() {
            return None;
        }
        Some(format!(
            "\n## Independent evidence read (TypeSafe Jev, calibrated — a first opinion, verify it yourself)\n{}\n",
            lines.join("\n")
        ))
    }
}
