//! Agent CLIs updated on the user's request (Providers & CLIs, ADR-153): each installed CLI with the version it has, the
//! newest one, and the one Agenium last verified. An update runs the CLI's own updater, never while one of its runs is
//! in progress (its program in use); then the CLI is found again and asked one short question, as a run would ask it.
//! When that check fails and the CLI can install a given version, the version it had is offered back. A version that
//! answered the check is kept (`<root>/cli-checks.json`): it is no longer shown as beyond the verified one, and an
//! updater that left the version as it was says the CLI is at its newest when its source cannot tell.

use agenium_cli_backend::updates;
use agenium_cli_backend::{CliInfo, CliKind};
use agenium_core::{BackendKind, Effort, ModelBinding};
use agenium_llm::Message;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::runtime::Runtime;
use crate::{OrchError, Result};

/// What Providers & CLIs shows about updating one CLI.
#[derive(Debug, Clone, Serialize)]
pub struct CliUpdateInfo {
    pub kind: CliKind,
    pub program: String,
    pub installed: Option<String>,
    /// The newest release, when the CLI's source says; `latest_error` why it could not be read.
    pub latest: Option<String>,
    pub latest_error: Option<String>,
    /// The version Agenium last checked end to end.
    pub verified: Option<&'static str>,
    /// A newer release than the one installed.
    pub update: bool,
    /// The release offered (or the one installed) is newer than the verified one.
    pub beyond_verified: bool,
    /// The CLI's updater can install a given version (a way back after a failed check).
    pub can_pin: bool,
    /// Runs of this CLI in progress (an update waits for them).
    pub busy: usize,
    /// The installed version answered Agenium's check after an update on this machine.
    pub checked: Option<CliCheck>,
}

/// A version that answered Agenium's check after an update here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CliCheck {
    pub version: String,
    pub at: String,
    /// The CLI's updater left the version as it was: it had nothing newer then.
    pub updater_had_none: bool,
}

/// What an update did, and the check after it.
#[derive(Debug, Clone, Serialize)]
pub struct CliUpdated {
    pub kind: CliKind,
    pub before: Option<String>,
    pub after: Option<String>,
    pub output: String,
    /// The short question asked through the CLI after the update.
    pub check_ok: bool,
    pub check: String,
    /// The version to go back to (the check failed and the updater can install it).
    pub rollback: Option<String>,
}

/// The state of one CLI, from what was found of it and what the root keeps. No command runs here, so the page's rules
/// (which version counts as checked, what a failed newest check says) are tested without a CLI on the machine.
fn state_of(
    c: &CliInfo,
    latest: Option<String>,
    latest_error: Option<String>,
    checked: Option<CliCheck>,
    busy: usize,
) -> CliUpdateInfo {
    let installed = c.version.clone();
    let verified = c.kind.verified_version();
    let update = matches!((&latest, &installed), (Some(l), Some(i)) if updates::newer(l, i));
    let beyond =
        |v: &Option<String>| matches!((v, verified), (Some(x), Some(ok)) if updates::newer(x, ok));
    CliUpdateInfo {
        kind: c.kind,
        program: c.kind.program().to_owned(),
        beyond_verified: (beyond(&installed) && checked.is_none()) || (update && beyond(&latest)),
        installed,
        latest,
        latest_error,
        verified,
        update,
        can_pin: c.kind.update_args(Some("0.0.0")).is_some(),
        busy,
        checked,
    }
}

/// The check kept for `program`, when it was made on the version installed now: a later version is not described by it.
fn kept_check(
    all: &BTreeMap<String, CliCheck>,
    program: &str,
    installed: Option<&str>,
) -> Option<CliCheck> {
    all.get(program)
        .filter(|k| installed == Some(k.version.as_str()))
        .cloned()
}

/// The CLI's updater left nothing newer to install: it ran to the newest and the version did not change. Only when the
/// user asked for no version in particular (asking for one that was already installed says nothing about the newest).
fn updater_had_none(requested: Option<&str>, before: Option<&str>, after: Option<&str>) -> bool {
    requested.is_none() && before.is_some() && before == after
}

/// A model of the CLI cheap to ask one question with: one whose name says it is small or free, else the first listed.
fn check_model(models: &[agenium_llm::ModelInfo]) -> Option<String> {
    const LIGHT: [&str; 6] = ["haiku", "flash", "free", "luna", "mini", "nano"];
    models
        .iter()
        .find(|m| LIGHT.iter().any(|w| m.id.to_lowercase().contains(w)))
        .or_else(|| models.first())
        .map(|m| m.id.clone())
}

impl App {
    fn cli_checks(&self) -> BTreeMap<String, CliCheck> {
        std::fs::read_to_string(self.layout().cli_checks())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Keeps the version of `program` that answered the check.
    pub fn record_cli_check(&self, program: &str, check: CliCheck) -> Result<()> {
        let mut all = self.cli_checks();
        all.insert(program.to_owned(), check);
        let text =
            serde_json::to_string_pretty(&all).map_err(|e| OrchError::Invalid(e.to_string()))?;
        Ok(agenium_config::write_atomic(
            &self.layout().cli_checks(),
            text.as_bytes(),
        )?)
    }

    /// Runs in progress on the agent CLI `program`.
    pub fn cli_runs_in_progress(&self, program: &str) -> Result<usize> {
        Ok(self
            .store()
            .read(agenium_store::core::runs::unfinished)?
            .iter()
            .filter(|r| r.provider == program)
            .count())
    }

    /// The update state of each installed CLI Agenium drives (`clis` as last found).
    pub async fn cli_updates(&self, clis: &[CliInfo]) -> Vec<CliUpdateInfo> {
        let mut out = Vec::new();
        let checks = self.cli_checks();
        for c in clis
            .iter()
            .filter(|c| c.path.is_some() && c.kind != CliKind::Devin)
        {
            let program = c.kind.program();
            let (latest, latest_error) = match updates::latest(c.kind, c.path.as_deref()).await {
                Ok(v) => (v, None),
                Err(e) => (None, Some(e)),
            };
            let checked = kept_check(&checks, program, c.version.as_deref());
            out.push(state_of(
                c,
                latest,
                latest_error,
                checked,
                self.cli_runs_in_progress(program).unwrap_or(0),
            ));
        }
        out
    }
}

impl Runtime {
    /// Updates the CLI `before` describes (to the newest version, or to `version`), finds it again and checks it with
    /// one short question.
    pub async fn update_cli(&self, before: &CliInfo, version: Option<&str>) -> Result<CliUpdated> {
        let kind = before.kind;
        let program = before
            .path
            .clone()
            .ok_or_else(|| OrchError::NotFound(format!("{} is not installed", kind.program())))?;
        let busy = self.app.cli_runs_in_progress(kind.program())?;
        if busy > 0 {
            return Err(OrchError::Conflict(format!(
                "{busy} run(s) of {} are in progress: update it once they end (its program is in use)",
                kind.program()
            )));
        }
        let done = updates::update(kind, &program, version)
            .await
            .map_err(OrchError::Invalid)?;
        let after = tokio::task::spawn_blocking(move || agenium_cli_backend::detect::detect(kind))
            .await
            .map_err(|e| OrchError::Invalid(e.to_string()))?;
        let models = self
            .app
            .model_cache()
            .models
            .get(&crate::model_cache::cli_key(kind.program()))
            .map(|l| l.value.clone())
            .unwrap_or_default();
        let (check_ok, check) = if !after.compatible {
            (
                false,
                format!(
                    "{} is not usable after the update: {}",
                    kind.program(),
                    after.note.clone().unwrap_or_default()
                ),
            )
        } else if let Some(model) = check_model(&models) {
            let binding = ModelBinding {
                provider: kind.program().to_owned(),
                model: model.clone(),
                effort: Effort::LOW,
                max_tokens: None,
            };
            let ask = "Reply with exactly this and nothing else: AGENIUM-OK";
            match self
                .side_chat(
                    "cli-check",
                    None,
                    BackendKind::Cli,
                    &binding,
                    "You answer in as few words as asked.",
                    vec![Message::user_text(ask)],
                    64,
                )
                .await
            {
                Ok(a) if a.text.contains("AGENIUM-OK") => (true, format!("{model} answered")),
                Ok(a) => (
                    false,
                    format!(
                        "{model} answered something else: {}",
                        a.text.chars().take(200).collect::<String>()
                    ),
                ),
                Err(e) => (false, format!("{model}: {e}")),
            }
        } else {
            (
                false,
                format!("no model of {} is listed to ask", kind.program()),
            )
        };
        if let (true, Some(v)) = (check_ok, &after.version) {
            let check = CliCheck {
                version: v.clone(),
                at: agenium_core::now_iso(),
                updater_had_none: updater_had_none(
                    version,
                    before.version.as_deref(),
                    after.version.as_deref(),
                ),
            };
            if let Err(e) = self.app.record_cli_check(kind.program(), check) {
                tracing::warn!(cli = kind.program(), error = %e, "the CLI's check was not kept");
            }
        }
        let rollback = match (&before.version, check_ok) {
            (Some(v), false)
                if kind.update_args(Some(v)).is_some() && after.version.as_ref() != Some(v) =>
            {
                Some(v.clone())
            }
            _ => None,
        };
        Ok(CliUpdated {
            kind,
            before: before.version.clone(),
            after: after.version,
            output: done.output,
            check_ok,
            check,
            rollback,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(kind: CliKind, version: &str) -> CliInfo {
        CliInfo {
            kind,
            path: Some(std::path::PathBuf::from(r"C:\fake\cli.exe")),
            version: Some(version.into()),
            login: agenium_cli_backend::Login::Unknown,
            compatible: true,
            note: None,
        }
    }

    fn check(version: &str, updater_had_none: bool) -> CliCheck {
        CliCheck {
            version: version.into(),
            at: "2026-11-02T01:00:00Z".into(),
            updater_had_none,
        }
    }

    /// A version that answered the check after an update here is not shown as beyond the one Agenium verified
    /// (opencode 2.2.5 against 2.1.2): the warning goes, the checked note stays.
    #[test]
    fn a_checked_version_is_not_beyond_the_verified_one() {
        let c = info(CliKind::Opencode, "2.2.5");
        let state = state_of(&c, Some("2.2.5".into()), None, None, 0);
        assert!(state.beyond_verified);
        assert!(state.checked.is_none());
        assert!(!state.update);

        let kept = check("2.2.5", false);
        let state = state_of(&c, Some("2.2.5".into()), None, Some(kept.clone()), 0);
        assert!(!state.beyond_verified);
        assert_eq!(state.checked, Some(kept));
    }

    /// The check is kept for the version it was made on: a later one is not described by it, and neither is another CLI.
    #[test]
    fn a_kept_check_counts_only_for_its_version() {
        let mut all = BTreeMap::new();
        all.insert("opencode".to_owned(), check("2.2.5", false));
        assert!(kept_check(&all, "opencode", Some("2.2.5")).is_some());
        assert!(kept_check(&all, "opencode", Some("2.2.6")).is_none());
        assert!(kept_check(&all, "opencode", None).is_none());
        assert!(kept_check(&all, "agy", Some("2.2.5")).is_none());
    }

    /// An update that left the version as it was had nothing newer — unless the user asked for that exact version.
    #[test]
    fn an_unchanged_update_had_nothing_newer() {
        assert!(updater_had_none(None, Some("1.4.12"), Some("1.4.12")));
        assert!(!updater_had_none(
            Some("1.4.12"),
            Some("1.4.12"),
            Some("1.4.12")
        ));
        assert!(!updater_had_none(None, Some("1.4.11"), Some("1.4.12")));
        assert!(!updater_had_none(None, None, Some("1.4.12")));
        assert!(!updater_had_none(None, Some("1.4.12"), None));
    }

    /// A newest its source cannot say (agy) is trusted to its own updater; a failed check is said with its reason
    /// (grok's own updater not finding npm), not hidden behind "newest unknown".
    #[test]
    fn an_unknown_newest_keeps_what_the_cli_said() {
        let c = info(CliKind::Grok, "1.0.41");
        let err = "grok update --check --json: program not found";
        let state = state_of(&c, None, Some(err.into()), None, 0);
        assert_eq!(state.latest, None);
        assert_eq!(state.latest_error.as_deref(), Some(err));
        assert!(!state.update);
        assert!(!state.beyond_verified);

        let c = info(CliKind::Agy, "1.4.12");
        let state = state_of(&c, None, None, Some(check("1.4.12", true)), 0);
        assert!(state.checked.as_ref().is_some_and(|k| k.updater_had_none));
        assert!(!state.beyond_verified);
    }

    /// An offered release newer than the one Agenium verified is still warned about, checked version or not.
    #[test]
    fn an_offer_beyond_the_verified_one_is_warned_about() {
        let c = info(CliKind::Opencode, "2.2.5");
        let state = state_of(
            &c,
            Some("2.3.0".into()),
            None,
            Some(check("2.2.5", false)),
            0,
        );
        assert!(state.update);
        assert!(state.beyond_verified);
    }

    #[test]
    fn the_check_asks_a_light_model() {
        let m = |id: &str| agenium_llm::ModelInfo {
            id: id.into(),
            ..Default::default()
        };
        assert_eq!(
            check_model(&[m("opus"), m("haiku")]).as_deref(),
            Some("haiku")
        );
        assert_eq!(
            check_model(&[
                m("opencode-go/deepseek-v4.1-flash"),
                m("opencode-go/beam-free")
            ])
            .as_deref(),
            Some("opencode-go/deepseek-v4.1-flash")
        );
        assert_eq!(check_model(&[m("grok-4.7")]).as_deref(), Some("grok-4.7"));
        assert_eq!(check_model(&[]), None);
    }
}
