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
            let installed = c.version.clone();
            let verified = c.kind.verified_version();
            let checked = checks
                .get(program)
                .filter(|k| installed.as_deref() == Some(k.version.as_str()))
                .cloned();
            let update =
                matches!((&latest, &installed), (Some(l), Some(i)) if updates::newer(l, i));
            let beyond = |v: &Option<String>| matches!((v, verified), (Some(x), Some(ok)) if updates::newer(x, ok));
            out.push(CliUpdateInfo {
                kind: c.kind,
                program: program.to_owned(),
                beyond_verified: (beyond(&installed) && checked.is_none())
                    || (update && beyond(&latest)),
                installed,
                latest,
                latest_error,
                verified,
                update,
                can_pin: c.kind.update_args(Some("0.0.0")).is_some(),
                busy: self.cli_runs_in_progress(program).unwrap_or(0),
                checked,
            });
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
                updater_had_none: version.is_none() && before.version.as_ref() == Some(v),
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
