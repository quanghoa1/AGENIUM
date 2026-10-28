//! When the judge is usable: a rejected key or an exhausted quota turns it off until the user acts; nothing else does.

use agenium_core::DomainEvent;
use agenium_llm::LlmError;
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::{OrchError, Result};

/// Whether the judge is usable now and, if not, why (persisted in `<root>/judge-health.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JudgeHealth {
    /// `true` when a key/quota failure turned the judge off until the user re-enables it.
    pub disabled: bool,
    pub reason: Option<String>,
    pub at: Option<String>,
}

/// A rejected key or an exhausted quota disables the judge (the user must act). A rate limit, a busy service or a bad
/// request does not: the client retried the busy ones, and the call just goes ahead without the judge. (Before, any
/// 429 turned the judge off until the user re-enabled it, and so did a 422 whose detail named a state key.)
pub(super) fn is_credential_or_quota(e: &LlmError) -> bool {
    matches!(e, LlmError::Config(m) if m.starts_with("TypeSafe key rejected") || m.starts_with("TypeSafe quota exhausted"))
}

impl App {
    pub fn judge_health(&self) -> JudgeHealth {
        std::fs::read_to_string(self.layout().judge_health())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub(super) fn set_judge_health(&self, h: &JudgeHealth) -> Result<()> {
        let path = self.layout().judge_health();
        std::fs::write(
            &path,
            serde_json::to_string_pretty(h).map_err(|e| OrchError::Invalid(e.to_string()))?,
        )
        .map_err(|e| OrchError::Invalid(format!("{}: {e}", path.display())))
    }

    /// Turns the judge off after a key/quota failure and tells the user to re-enter the key and re-enable it.
    pub(super) fn disable_judge(&self, reason: String) -> Result<()> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only a rejected key or an exhausted quota turns the judge off; a rate limit, a busy service or a bad request
    /// (even one whose detail names a state key) does not.
    #[test]
    fn only_key_and_quota_failures_disable_the_judge() {
        let cfg = |m: &str| LlmError::Config(m.to_owned());
        assert!(is_credential_or_quota(&cfg(
            "TypeSafe key rejected: bad key"
        )));
        assert!(is_credential_or_quota(&cfg(
            "TypeSafe quota exhausted: add credit"
        )));
        assert!(!is_credential_or_quota(&cfg(
            "TypeSafe request invalid: unknown state key `report`"
        )));
        assert!(!is_credential_or_quota(&LlmError::Network(
            "TypeSafe rate limited: slow down".into()
        )));
        assert!(!is_credential_or_quota(&LlmError::Network(
            "TypeSafe busy (HTTP 503): x".into()
        )));
        assert!(!is_credential_or_quota(&LlmError::Timeout));
    }
}
