//! Which judge keys may be used (the keys take turns, `client`). A key out of quota is set aside and the user is told
//! which one, by the name they gave it; it stays out until the user adds credit and re-enables it — never retried by
//! itself. A rejected key rests and is tried again by itself: after 30 minutes, then after an hour if it fails again.
//! The request either failed goes on with another key at once. With no key left the judge is off, and the work goes
//! on without it (fail-open). A failure that is not the key's — a bad or too large request, a timeout, the service
//! busy or down — sets no key aside: that request goes on without the judge, and the judge pauses a minute (audit
//! M15, 2026-10-31: one request too large for the service set every key aside for up to an hour).

use std::collections::BTreeMap;

use agenium_core::DomainEvent;
use agenium_llm::LlmError;
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::{OrchError, Result};

/// How long a failing key rests: the first time, and from the second failure in a row on.
const FIRST_REST: chrono::Duration = chrono::Duration::minutes(30);
const LATER_REST: chrono::Duration = chrono::Duration::minutes(60);

/// A key set aside, and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyOff {
    /// Out of quota (`true`: out until the user re-enables it) or failing (`false`: rests until `until`).
    pub quota: bool,
    pub reason: String,
    pub at: String,
    /// A failing key is tried again after this time (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Failures in a row (the rest grows from 30 minutes to an hour).
    #[serde(default)]
    pub failures: u32,
}

impl KeyOff {
    /// The key may be used again now (a failing key whose rest is over).
    fn over(&self) -> bool {
        !self.quota
            && self
                .until
                .as_deref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .is_none_or(|u| chrono::Utc::now() >= u)
    }
}

/// The judge's keys that are set aside (persisted in `<root>/judge-health.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JudgeHealth {
    /// Every key is out of quota: the judge is off until the user re-enables one.
    pub disabled: bool,
    /// Why the last key was set aside.
    pub reason: Option<String>,
    pub at: Option<String>,
    /// Set-aside keys by name.
    #[serde(default)]
    pub keys_off: BTreeMap<String, KeyOff>,
}

/// What a failure means for the key that made it.
#[derive(Debug, PartialEq)]
pub(super) enum Stop {
    /// Out of quota: set aside until the user adds credit and re-enables it.
    Quota,
    /// Rejected: the key rests and is tried again by itself.
    Failing,
}

/// What a failure means for its key: `None` when it is not the key's (a bad or too large request, a timeout, the
/// service busy or down, a rate limit), which another key would meet the same way. (Before, any 429 turned the whole
/// judge off until the user re-enabled it, and so did a 422 whose detail named a state key; then a timeout set every
/// key aside in turn.)
pub(super) fn stop_for(e: &LlmError) -> Option<Stop> {
    match e {
        LlmError::Config(m) if m.starts_with("TypeSafe quota exhausted") => Some(Stop::Quota),
        LlmError::Config(m) if m.starts_with("TypeSafe key rejected") => Some(Stop::Failing),
        _ => None,
    }
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

    /// The configured keys that may be used now, in order (a failing key whose rest is over is back).
    pub(super) fn keys_in_use(&self) -> Vec<String> {
        let off = self.judge_health().keys_off;
        self.config()
            .judge
            .key_list()
            .into_iter()
            .map(|k| k.name)
            .filter(|n| off.get(n).is_none_or(KeyOff::over))
            .collect()
    }

    /// Sets key `name` aside after `e`: out of quota until the user re-enables it (the user is told which key, and when
    /// no key is left that the judge is off), or failing for a rest of 30 minutes, then an hour.
    pub(super) fn set_key_aside(&self, name: &str, stop: Stop, e: &LlmError) -> Result<()> {
        let mut h = self.judge_health();
        let quota = stop == Stop::Quota;
        let failures = match h.keys_off.get(name) {
            Some(o) if o.quota => return Ok(()),
            Some(o) if !quota => o.failures + 1,
            _ => 1,
        };
        let until = (!quota).then(|| {
            let rest = if failures > 1 { LATER_REST } else { FIRST_REST };
            (chrono::Utc::now() + rest).to_rfc3339()
        });
        h.keys_off.insert(
            name.to_owned(),
            KeyOff {
                quota,
                reason: e.to_string(),
                at: agenium_core::now_iso(),
                until: until.clone(),
                failures,
            },
        );
        let keys = self.config().judge.key_list();
        h.disabled = !keys.is_empty()
            && keys
                .iter()
                .all(|k| h.keys_off.get(&k.name).is_some_and(|o| o.quota));
        h.reason = Some(format!("key {name:?}: {e}"));
        h.at = Some(agenium_core::now_iso());
        self.set_judge_health(&h)?;
        tracing::warn!(key = name, quota, ?until, "judge key set aside");
        if !quota {
            // It comes back by itself; the Settings page shows when.
            return Ok(());
        }
        let now = if h.disabled {
            " Every Jev key is now out of quota: the judge is off, and Agenium works without it until you re-enable a \
             key."
                .to_owned()
        } else {
            let left = self.keys_in_use();
            if left.is_empty() {
                " No other key is usable right now: Agenium works without the judge until one is."
                    .to_owned()
            } else {
                format!(" The other keys go on: {}.", left.join(", "))
            }
        };
        self.store().commit(None, "system", |w| -> Result<()> {
            w.emit(DomainEvent::NeedsUser {
                ticket: None,
                reason: format!(
                    "Jev key \"{name}\" is out of quota: add credit to it, then re-enable it in Settings → Judge.{now}"
                ),
            })?;
            Ok(())
        })?;
        Ok(())
    }

    /// Brings key `name` (or every key) back into use; the judge is on again when a key is.
    pub(super) fn clear_key_aside(&self, name: Option<&str>) -> Result<()> {
        let mut h = self.judge_health();
        match name {
            Some(n) => {
                h.keys_off.remove(n);
            }
            None => h.keys_off.clear(),
        }
        h.disabled = false;
        h.reason = None;
        self.set_judge_health(&h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An exhausted quota sets the key aside for the user; a rejected key or a failing service makes it rest; a bad
    /// request (even one whose detail names a state key) is not the key's fault.
    #[test]
    fn what_a_failure_means_for_its_key() {
        let cfg = |m: &str| LlmError::Config(m.to_owned());
        assert_eq!(
            stop_for(&cfg("TypeSafe quota exhausted: add credit")),
            Some(Stop::Quota)
        );
        assert_eq!(
            stop_for(&cfg("TypeSafe key rejected: bad key")),
            Some(Stop::Failing)
        );
        // Not the key's: another key would meet them the same way (audit M15).
        assert_eq!(
            stop_for(&LlmError::Network("TypeSafe busy (HTTP 503): x".into())),
            None
        );
        assert_eq!(stop_for(&LlmError::Timeout), None);
        assert_eq!(
            stop_for(&LlmError::Network("TypeSafe HTTP 413: too large".into())),
            None
        );
        assert_eq!(
            stop_for(&cfg("TypeSafe request invalid: unknown state key `report`")),
            None
        );
    }

    #[test]
    fn a_failing_key_is_back_after_its_rest_a_key_out_of_quota_is_not() {
        let past = (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
        let future = (chrono::Utc::now() + chrono::Duration::minutes(29)).to_rfc3339();
        let off = |quota: bool, until: Option<String>| KeyOff {
            quota,
            reason: String::new(),
            at: String::new(),
            until,
            failures: 1,
        };
        assert!(off(false, Some(past)).over());
        assert!(!off(false, Some(future)).over());
        assert!(!off(true, None).over());
    }
}
