//! Which models an agent can pick, by backend: an API provider lists what its API serves (OpenCode's free models
//! answer only to OpenCode itself, so they are not offered there), a vendor CLI lists what that CLI runs with the
//! user's login (codex `model/list`, `agy models`, Claude Code's aliases and ids, the user's OpenCode account for the
//! opencode CLI). What they list is kept in the model cache (`model_cache`). Each model comes with its effort levels where its CLI names them (Codex's `model/list`,
//! Antigravity's variants) or they are known for it (Claude, ADR-114). Nothing here costs quota.

use agenium_cli_backend::{CliInfo, CliKind, Login};
use agenium_llm::ModelInfo;

use crate::app::App;

impl App {
    /// Whether the API of provider `provider` serves `model`.
    pub fn api_serves(&self, provider: &str, model: &str) -> bool {
        let Some(cfg) = self.config().provider(provider).cloned() else {
            return true;
        };
        let url = cfg.base_url.as_deref().unwrap_or("").to_ascii_lowercase();
        // Zen's free models answer only inside OpenCode; the Go subscription's own free ones (`beam-free`) answer over
        // its API like its other models (probed 2026-10-31).
        if !url.contains("opencode.ai") || url.contains("/zen/go") {
            return true;
        }
        let free_in_catalogue = self.prices().catalogue.providers.iter().any(|p| {
            p.id.starts_with("opencode") && p.models.get(model).is_some_and(|x| x.is_free())
        });
        !(model.ends_with("-free") || free_in_catalogue)
    }
}

/// [`cli_models_of`] of several CLIs at once (each listing starts its CLI, some call a server): by CLI program name.
pub async fn cli_models_each<'a>(
    app: &App,
    clis: impl Iterator<Item = &'a CliInfo>,
) -> Vec<(&'static str, Option<Result<Vec<ModelInfo>, String>>)> {
    crate::runtime::futures_join_all(
        clis.map(|c| async move { (c.kind.program(), cli_models_of(app, c).await) }),
    )
    .await
}

/// One CLI's models; `None` when it lists none (not installed, too old, signed out, or not driven).
pub async fn cli_models_of(app: &App, c: &CliInfo) -> Option<Result<Vec<ModelInfo>, String>> {
    if !c.compatible || c.login == Login::NotLoggedIn {
        return None;
    }
    let scratch = app.layout().agents().join(".models-probe");
    let program = c.path.as_ref()?;
    // OpenCode names no levels (its agents keep the CLI's own setting); Grok and Claude Code run their vendor's
    // models, whose levels are known per model.
    let ids = |v: Vec<String>| {
        v.into_iter()
            .map(|id| {
                let mut m = ModelInfo {
                    id,
                    ..Default::default()
                };
                agenium_llm::effort::fill(&mut m);
                m
            })
            .collect()
    };
    let models = match c.kind {
        CliKind::Codex => agenium_cli_backend::codex::models(program, &scratch).await,
        CliKind::Agy => agenium_cli_backend::agy::models(program).await,
        CliKind::Claude => Ok(ids(agenium_cli_backend::claude::MODELS
            .map(str::to_owned)
            .to_vec())),
        // What the user's own OpenCode account offers (its Go subscription, Zen); without a login, OpenCode's free
        // models from the price catalogue (they need none).
        CliKind::Opencode => {
            let account = agenium_cli_backend::opencode::account_models().await;
            Ok(if account.is_empty() {
                app.opencode_free_models()
                    .into_iter()
                    .map(|id| ModelInfo {
                        id,
                        ..Default::default()
                    })
                    .collect()
            } else {
                account
            })
        }
        CliKind::Grok => agenium_cli_backend::grok::models(program).await.map(ids),
        CliKind::Devin => return None,
    };
    Some(models)
}
