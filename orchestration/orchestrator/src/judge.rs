//! The optional TypeSafe "Jev" judge (ADR-75): a cheap, calibrated second opinion, off until `[judge]` is set.
//! Agenium never uses it as an agent's model. Its one wired use is an independent evidence read before a review run
//! (`review_assist`): one batched request, a Noul per acceptance criterion, so the Lead's expensive model starts the
//! review with a token-light first opinion instead of forming one from scratch.

use agenium_core::Ticket;
use agenium_llm::systemone::{Judge, Judging};

use crate::app::App;
use crate::{OrchError, Result};

/// A Noul at or above this reads as "met", at or below `1 - this` as "not met", between as "unsure" (decision 35).
const MET: f64 = 0.7;

impl App {
    /// The configured judge, or `None` when `[judge]` is off. `Err` only when it is on but misconfigured.
    pub fn judge(&self) -> Result<Option<Judge>> {
        let j = self.config().judge;
        if !j.enabled() {
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

    /// An independent first read of `report` against `t`'s acceptance criteria, for the review task (ADR-75).
    /// `None` when the judge is off, `review_assist` is off, there are no criteria, or the call failed (the review
    /// then proceeds on the Lead's own read).
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
                tracing::warn!(error = %e, "judge evidence read failed");
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
