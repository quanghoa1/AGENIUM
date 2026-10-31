//! What each model source offers, kept in the root (`model-cache.json`): the agent CLIs found on this machine, and the
//! models each CLI (`cli:<program>`) and each API provider (`api:<id>`) lists, with when (user, 2026-10-31). The app
//! reads them from here; a source is listed again when the user reloads it, the first time it is seen (a new install,
//! provider or CLI), and on its own once its list is [`MAX_AGE_DAYS`] old. Before, every CLI was started again to find
//! it and list its models every five minutes the app was open, and an API provider listed its models each time a list
//! was shown.

use std::collections::BTreeMap;
use std::sync::Mutex;

use agenium_cli_backend::CliInfo;
use agenium_llm::ModelInfo;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::{OrchError, Result};

/// A list this old is listed again on its own, once.
pub const MAX_AGE_DAYS: i64 = 7;

/// A listing that failed is not tried again on its own sooner than this (the user's reload always is).
pub const RETRY_AFTER_HOURS: i64 = 24;

/// One source's last listing: what it gave, when it last succeeded, and the last failure since.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Cached<T> {
    pub value: T,
    /// When it was last listed successfully (RFC 3339); `None` = never.
    #[serde(default)]
    pub at: Option<String>,
    /// The last attempt's error, when it failed (the value is the last good one).
    #[serde(default)]
    pub error: Option<String>,
    /// When it was last tried (RFC 3339).
    #[serde(default)]
    pub tried_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelCache {
    /// The agent CLIs found on this machine.
    #[serde(default)]
    pub clis: Option<Cached<Vec<CliInfo>>>,
    /// The models of each source, by key ([`cli_key`], [`api_key`]).
    #[serde(default)]
    pub models: BTreeMap<String, Cached<Vec<ModelInfo>>>,
}

pub fn cli_key(program: &str) -> String {
    format!("cli:{program}")
}

pub fn api_key(provider: &str) -> String {
    format!("api:{provider}")
}

/// Whether a source is listed again on its own at `now`: never listed, or its list is [`MAX_AGE_DAYS`] old, and no
/// attempt failed in the last [`RETRY_AFTER_HOURS`].
pub fn due<T>(entry: Option<&Cached<T>>, now: DateTime<Utc>) -> bool {
    let Some(e) = entry else {
        return true;
    };
    let old = |at: &Option<String>, d: Duration| {
        at.as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .is_none_or(|t| now - t.with_timezone(&Utc) >= d)
    };
    old(&e.at, Duration::days(MAX_AGE_DAYS)) && old(&e.tried_at, Duration::hours(RETRY_AFTER_HOURS))
}

/// `entry` after an attempt at `now`: a success replaces the value, a failure keeps the last good one and says why.
pub fn record<T>(
    entry: Option<Cached<T>>,
    got: std::result::Result<T, String>,
    now: DateTime<Utc>,
) -> Cached<T>
where
    T: Default,
{
    let now = now.to_rfc3339();
    match (entry, got) {
        (_, Ok(value)) => Cached {
            value,
            at: Some(now.clone()),
            error: None,
            tried_at: Some(now),
        },
        (old, Err(e)) => {
            let old = old.unwrap_or_default();
            Cached {
                value: old.value,
                at: old.at,
                error: Some(e),
                tried_at: Some(now),
            }
        }
    }
}

/// One writer at a time: listings of several sources finish together.
static LOCK: Mutex<()> = Mutex::new(());

impl App {
    /// The cache as stored (empty when there is none or it cannot be read: everything is then listed again).
    pub fn model_cache(&self) -> ModelCache {
        std::fs::read(self.layout().model_cache())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Reads, changes and writes the cache under one lock.
    pub fn update_model_cache<R>(&self, f: impl FnOnce(&mut ModelCache) -> R) -> Result<R> {
        let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut c = self.model_cache();
        let r = f(&mut c);
        let text = serde_json::to_vec_pretty(&c).map_err(|e| OrchError::Invalid(e.to_string()))?;
        agenium_config::write_atomic(&self.layout().model_cache(), &text)?;
        Ok(r)
    }

    /// Records a listing of the models of `key`; returns the entry as stored.
    pub fn record_models(
        &self,
        key: &str,
        got: std::result::Result<Vec<ModelInfo>, String>,
    ) -> Result<Cached<Vec<ModelInfo>>> {
        self.update_model_cache(|c| {
            let e = record(c.models.remove(key), got, Utc::now());
            c.models.insert(key.to_owned(), e.clone());
            e
        })
    }

    /// Records the agent CLIs found now.
    pub fn record_clis(&self, found: Vec<CliInfo>) -> Result<Cached<Vec<CliInfo>>> {
        self.update_model_cache(|c| {
            let e = record(c.clis.take(), Ok(found), Utc::now());
            c.clis = Some(e.clone());
            e
        })
    }

    /// Forgets a source (a provider deleted).
    pub fn forget_models(&self, key: &str) -> Result<()> {
        self.update_model_cache(|c| {
            c.models.remove(key);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// Listed again on its own: never listed, or a week old; not while a failure is fresher than a day. A failure keeps
    /// the last good list.
    #[test]
    fn a_list_is_refreshed_once_a_week_and_a_failure_waits_a_day() {
        let now = at("2026-10-31T12:00:00Z");
        assert!(due::<Vec<String>>(None, now));
        let ok = record(None, Ok(vec!["m1".to_owned()]), at("2026-10-30T12:00:00Z"));
        assert!(!due(Some(&ok), now), "a day old");
        assert!(due(Some(&ok), at("2026-11-06T12:00:00Z")), "a week old");
        assert!(!due(Some(&ok), at("2026-11-06T11:59:00Z")));

        // The week is up and the listing fails: the old list stays, with the error; not tried again for a day.
        let failed = record(
            Some(ok.clone()),
            Err("offline".into()),
            at("2026-11-06T12:00:00Z"),
        );
        assert_eq!(failed.value, vec!["m1".to_owned()]);
        assert_eq!(
            (failed.at.clone(), failed.error.as_deref()),
            (ok.at.clone(), Some("offline"))
        );
        assert!(!due(Some(&failed), at("2026-11-07T11:00:00Z")));
        assert!(due(Some(&failed), at("2026-11-07T12:00:00Z")));

        // Never listed and failing: a day between attempts too.
        let never = record::<Vec<String>>(None, Err("no login".into()), at("2026-10-31T12:00:00Z"));
        assert!(never.at.is_none() && never.value.is_empty());
        assert!(!due(Some(&never), at("2026-10-31T20:00:00Z")));
        assert!(due(Some(&never), at("2026-11-01T12:00:00Z")));

        // A success clears the error.
        let back = record(Some(failed), Ok(vec!["m2".to_owned()]), now);
        assert_eq!((back.value, back.error), (vec!["m2".to_owned()], None));
    }
}
