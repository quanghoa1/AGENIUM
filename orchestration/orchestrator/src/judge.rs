//! The optional TypeSafe "Jev" judge (ADR-75, ADR-81): a cheap, calibrated second opinion, off until `[judge]` is
//! set. Agenium never uses it as an agent's model, and it runs in two modes — with Jev and without. Without Jev (no
//! config, or after a failure) every judge call simply returns nothing and the work proceeds on the agent's own
//! judgment, so Agenium always works. A call that fails on the key or the quota turns the judge off and asks the
//! user to re-enter the key and re-enable it; a run error does not.
//!
//! Its wired use is an independent evidence read before a review run (`review_assist`): one batched request, a Noul
//! per acceptance criterion, a token-light first opinion for the Lead's expensive model.

use agenium_core::{DomainEvent, Ticket};
use agenium_llm::systemone::{Judge, Judging};
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
