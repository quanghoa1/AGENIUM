//! Which models an agent can pick, by backend: an API provider lists what its API serves (OpenCode's free models
//! answer only to OpenCode itself, so they are not offered there), a vendor CLI lists what that CLI runs with the
//! user's login (codex `model/list`, `agy models`, Claude Code's aliases and ids, OpenCode's free models for the
//! opencode CLI). Nothing here costs quota.

use std::collections::BTreeMap;

use agenium_cli_backend::{CliInfo, CliKind, Login};

use crate::app::App;

impl App {
    /// Whether the API of provider `provider` serves `model`.
    pub fn api_serves(&self, provider: &str, model: &str) -> bool {
        let Some(cfg) = self.config().provider(provider).cloned() else {
            return true;
        };
        let opencode = cfg
            .base_url
            .as_deref()
            .is_some_and(|u| u.to_ascii_lowercase().contains("opencode.ai"));
        if !opencode {
            return true;
        }
        let free_in_catalogue = self.prices().catalogue.providers.iter().any(|p| {
            p.id.starts_with("opencode") && p.models.get(model).is_some_and(|x| x.is_free())
        });
        !(model.ends_with("-free") || free_in_catalogue)
    }
}

/// The models each installed, signed-in CLI that Agenium drives can run (by CLI program name).
pub async fn cli_models(app: &App, clis: &[CliInfo]) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    let scratch = app.layout().agents().join(".models-probe");
    for c in clis
        .iter()
        .filter(|c| c.compatible && c.login != Login::NotLoggedIn)
    {
        let Some(program) = &c.path else { continue };
        let models = match c.kind {
            CliKind::Codex => agenium_cli_backend::codex::models(program, &scratch).await,
            CliKind::Agy => agenium_cli_backend::agy::models(program).await,
            CliKind::Claude => Ok(agenium_cli_backend::claude::MODELS
                .map(str::to_owned)
                .to_vec()),
            CliKind::Opencode => Ok(app.opencode_free_models()),
            CliKind::Grok => agenium_cli_backend::grok::models(program).await,
            CliKind::Devin => continue,
        };
        match models {
            Ok(m) => {
                out.insert(c.kind.program().to_owned(), m);
            }
            Err(e) => tracing::warn!(cli = c.kind.program(), error = %e, "models not listed"),
        }
    }
    out
}
