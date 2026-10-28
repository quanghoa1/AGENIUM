//! The judge as the user sets it: endpoint, model, its key (in the vault, never in the config), which uses are on,
//! and the test call that re-enables it.

use agenium_llm::systemone::Judging;
use serde::Deserialize;

use super::client::JudgeActivity;
use super::health::JudgeHealth;
use crate::app::App;
use crate::{OrchError, Result};

/// Vault name of the judge's API key.
pub const JUDGE_SECRET: &str = "judge/typesafe";

/// The judge as the user sets it in the app: endpoint, model, the key (kept as is when not given) and what it may do.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct JudgeInput {
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_key: bool,
    #[serde(default)]
    pub review_assist: bool,
    #[serde(default)]
    pub skill_hint: bool,
    #[serde(default)]
    pub memory_gate: bool,
    #[serde(default)]
    pub skill_quality: bool,
    #[serde(default)]
    pub done_check: bool,
    #[serde(default)]
    pub trim_output: bool,
    #[serde(default)]
    pub shadow: bool,
}

/// Whether `url` is plain HTTP to this machine (localhost, 127.0.0.1 or [::1], any port): the only endpoint the judge
/// may use without TLS. The host is compared whole, so `http://localhost.example.com` or `http://localhost@host` is not
/// local.
fn local_http(url: &str) -> bool {
    let Some(rest) = url
        .get(..7)
        .filter(|s| s.eq_ignore_ascii_case("http://"))
        .map(|_| &url[7..])
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return false;
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => match v6.split_once(']') {
            Some((h, port)) if port.is_empty() || port.starts_with(':') => h,
            _ => return false,
        },
        None => authority.split(':').next().unwrap_or_default(),
    };
    ["localhost", "127.0.0.1", "::1"]
        .iter()
        .any(|l| host.eq_ignore_ascii_case(l))
}

impl App {
    /// Sets the judge (ADR-75): an empty endpoint turns it off. A new key goes to the vault, never to the config; a
    /// change clears an earlier failure so the judge is tried again.
    pub fn set_judge(&self, j: &JudgeInput) -> Result<agenium_config::JudgeSection> {
        let base_url = j.base_url.trim().trim_end_matches('/').to_owned();
        let local = local_http(&base_url);
        if !base_url.is_empty() && !base_url.starts_with("https://") && !local {
            return Err(OrchError::Invalid(format!(
                "judge endpoint {base_url:?}: use https://"
            )));
        }
        let old = self.config().judge;
        let key_ref = match (&j.api_key, j.clear_key) {
            (Some(k), _) if !k.trim().is_empty() => {
                self.secrets().set(JUDGE_SECRET, k.trim().as_bytes())?;
                JUDGE_SECRET.to_owned()
            }
            (_, true) => {
                let _ = self.secrets().delete(JUDGE_SECRET);
                String::new()
            }
            _ => old.api_key_ref.clone(),
        };
        let next = agenium_config::JudgeSection {
            base_url,
            model: j.model.trim().to_owned(),
            api_key_ref: key_ref,
            review_assist: j.review_assist,
            skill_hint: j.skill_hint,
            memory_gate: j.memory_gate,
            skill_quality: j.skill_quality,
            done_check: j.done_check,
            trim_output: j.trim_output,
            shadow: j.shadow,
        };
        if next.enabled() && next.api_key_ref.is_empty() && !local {
            return Err(OrchError::Invalid("the judge needs its API key".into()));
        }
        self.update_config(|c| c.judge = next.clone())?;
        self.set_judge_health(&JudgeHealth::default())?;
        Ok(next)
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
        let started = std::time::Instant::now();
        let r = judge.ask_full(&probe).await;
        let ms = started.elapsed().as_millis() as u64;
        let mut a = JudgeActivity::now("test", None, ms);
        match r {
            Ok(asked) => {
                a.outcome = "answered the test question".into();
                a.ok = true;
                a.usage = asked.usage;
                self.log_activity(a);
                Ok(())
            }
            Err(e) => {
                a.outcome = format!("failed: {e}");
                self.log_activity(a);
                self.set_judge_health(&JudgeHealth {
                    disabled: true,
                    reason: Some(e.to_string()),
                    at: Some(agenium_core::now_iso()),
                })?;
                Err(OrchError::Invalid(format!("the judge still fails: {e}")))
            }
        }
    }
}
