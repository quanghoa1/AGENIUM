//! The judge as the user sets it: endpoint, model, its key (in the vault, never in the config), which uses are on,
//! and the test call that re-enables it.

use agenium_llm::systemone::Judging;
use serde::Deserialize;

use super::client::JudgeActivity;
use super::health::stop_for;
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
    pub find_code: bool,
    #[serde(default)]
    pub shadow: bool,
}

/// The vault entry of the judge key named `name`.
fn key_secret(name: &str) -> String {
    let slug: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    format!("judge/key/{}", slug.trim_matches('-'))
}

impl App {
    /// Sets the judge (ADR-75): endpoint, model and which uses are on (an empty endpoint turns it off). `api_key` is the
    /// key named "default" (the form of a single key); named keys are added with [`App::add_judge_key`]. Keys go to the
    /// vault, never to the config. Without a key the judge simply stays off (fail-open).
    pub fn set_judge(&self, j: &JudgeInput) -> Result<agenium_config::JudgeSection> {
        let base_url = j.base_url.trim().trim_end_matches('/').to_owned();
        let local = agenium_config::config::local_endpoint(&base_url);
        if !base_url.is_empty() && !base_url.starts_with("https://") && !local {
            return Err(OrchError::Invalid(format!(
                "judge endpoint {base_url:?}: use https://"
            )));
        }
        let old = self.config().judge;
        let key_ref = match (&j.api_key, j.clear_key) {
            (Some(k), _) if !k.trim().is_empty() => {
                self.secrets().set(JUDGE_SECRET, k.trim().as_bytes())?;
                self.clear_key_aside(Some(agenium_config::JudgeSection::LEGACY_KEY))?;
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
            keys: old.keys.clone(),
            review_assist: j.review_assist,
            skill_hint: j.skill_hint,
            memory_gate: j.memory_gate,
            skill_quality: j.skill_quality,
            done_check: j.done_check,
            trim_output: j.trim_output,
            find_code: j.find_code,
            shadow: j.shadow,
        };
        self.update_config(|c| c.judge = next.clone())?;
        Ok(next)
    }

    /// Adds (or replaces the value of) the judge key `name`; keys are used in the order they were added. A key that was
    /// set aside comes back into use.
    pub fn add_judge_key(&self, name: &str, value: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 40 {
            return Err(OrchError::Invalid(
                "a key needs a name of 1 to 40 characters".into(),
            ));
        }
        if value.trim().is_empty() {
            return Err(OrchError::Invalid(format!("key {name:?} has no value")));
        }
        if name == agenium_config::JudgeSection::LEGACY_KEY
            && !self.config().judge.api_key_ref.is_empty()
        {
            self.secrets().set(JUDGE_SECRET, value.trim().as_bytes())?;
        } else {
            let secret = key_secret(name);
            if self
                .config()
                .judge
                .keys
                .iter()
                .any(|k| k.name != name && k.secret_ref == secret)
            {
                return Err(OrchError::Invalid(format!(
                    "key {name:?} is too close to the name of another key"
                )));
            }
            self.secrets().set(&secret, value.trim().as_bytes())?;
            self.update_config(|c| {
                if !c.judge.keys.iter().any(|k| k.name == name) {
                    c.judge.keys.push(agenium_config::JudgeKey {
                        name: name.to_owned(),
                        secret_ref: secret.clone(),
                    });
                }
            })?;
        }
        self.clear_key_aside(Some(name))
    }

    /// Removes the judge key `name` and its value.
    pub fn remove_judge_key(&self, name: &str) -> Result<()> {
        let j = self.config().judge;
        let Some(k) = j.key_list().into_iter().find(|k| k.name == name) else {
            return Err(OrchError::NotFound(format!("judge key {name:?}")));
        };
        let _ = self.secrets().delete(&k.secret_ref);
        self.update_config(|c| {
            if name == agenium_config::JudgeSection::LEGACY_KEY
                && c.judge.api_key_ref == k.secret_ref
            {
                c.judge.api_key_ref.clear();
            }
            c.judge.keys.retain(|x| x.name != name);
        })?;
        let mut h = self.judge_health();
        h.keys_off.remove(name);
        self.set_judge_health(&h)
    }

    /// Brings key `name` (or every key) back into use — after the user added credit or re-entered it — and checks it
    /// with a trivial request. On failure the key is set aside again and the error returned.
    pub async fn enable_judge(&self, name: Option<&str>) -> Result<()> {
        self.clear_key_aside(name)?;
        let pick = name
            .map(super::client::Pick::Named)
            .or(Some(super::client::Pick::First));
        let Some((key, judge)) = self.judge_with_key(pick)? else {
            return Err(OrchError::Invalid(
                "the judge is not configured: set the endpoint, the model and a key first".into(),
            ));
        };
        let probe = Judging::default()
            .state_text("x", "ok")
            .noul("q", "Does `x` say ok?");
        let started = std::time::Instant::now();
        let r = judge.ask_full(&probe).await;
        let ms = started.elapsed().as_millis() as u64;
        let mut a = JudgeActivity::now("test", None, ms);
        a.key = Some(key.clone());
        match r {
            Ok(asked) => {
                a.outcome = format!("key {key:?} answered the test question");
                a.ok = true;
                a.usage = asked.usage;
                self.log_activity(a);
                Ok(())
            }
            Err(e) => {
                a.outcome = format!("failed: {e}");
                self.log_activity(a);
                if let Some(stop) = stop_for(&e) {
                    let _ = self.set_key_aside(&key, stop, &e);
                }
                Err(OrchError::Invalid(format!("key {key:?} still fails: {e}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_map_to_vault_entries() {
        assert_eq!(key_secret("Team key #2"), "judge/key/team-key--2");
        assert_eq!(key_secret("  main "), "judge/key/main");
    }
}
