//! Provider sources for the runtime: how a provider is built from config + the secret store (`ConfiguredProviders`)
//! or injected (`StaticProviders`). Split out of `runtime/mod.rs`.

use std::sync::{Arc, Mutex};

use agenium_core::Effort;
use agenium_llm::{Provider, ProviderSpec};

use crate::app::App;
use crate::{OrchError, Result};

/// Supplies providers by id (from `config.toml` + the secret store, or injected in tests/evals).
pub trait ProviderSource: Send + Sync {
    fn provider(&self, id: &str) -> Result<Arc<dyn Provider>>;
    /// Drops what was built for `id` (its configuration or key changed).
    fn forget(&self, _id: &str) {}
}

/// Providers built from configuration; API keys are read from the secret store at first use.
pub struct ConfiguredProviders {
    app: Arc<App>,
    cache: Mutex<std::collections::HashMap<String, Arc<dyn Provider>>>,
}

impl ConfiguredProviders {
    pub fn new(app: Arc<App>) -> Self {
        Self {
            app,
            cache: Mutex::new(Default::default()),
        }
    }
}

impl ProviderSource for ConfiguredProviders {
    fn provider(&self, id: &str) -> Result<Arc<dyn Provider>> {
        if let Some(p) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(id) {
            return Ok(p.clone());
        }
        let cfg = self.app.config();
        let pc = cfg.provider(id).ok_or_else(|| {
            OrchError::NotFound(format!("provider {id:?} is not configured in config.toml"))
        })?;
        let key = if pc.api_key_ref.is_empty() {
            None
        } else {
            let s = self.app.secret(&pc.api_key_ref)?.ok_or_else(|| {
                OrchError::NotFound(format!(
                    "secret {:?} for provider {id} is not set",
                    pc.api_key_ref
                ))
            })?;
            Some(s.expose_str()?.to_owned())
        };
        let kind = match pc.kind {
            agenium_config::ProviderKind::Anthropic => "anthropic",
            agenium_config::ProviderKind::Gemini => "gemini",
            agenium_config::ProviderKind::OpenaiCompat => "openai_compat",
            agenium_config::ProviderKind::Mock => {
                return Err(OrchError::Invalid(
                    "mock providers are injected by tests, not configured".into(),
                ))
            }
        };
        // The user's headers (secret ones from the vault) and the effort levels given to models by hand.
        let secret = |n: &str| {
            self.app
                .secret(n)
                .ok()
                .flatten()
                .and_then(|s| s.expose_str().ok().map(str::to_owned))
        };
        let mut headers = Vec::new();
        for (k, v) in &pc.headers {
            let v = agenium_mcp::resolve(v, &secret, &|_| None)
                .map_err(|e| OrchError::NotFound(format!("provider {id}: header {k}: {e}")))?;
            headers.push((k.clone(), v));
        }
        let efforts = pc
            .model_caps
            .iter()
            .filter_map(|c| {
                let levels: Vec<Effort> = c
                    .efforts
                    .as_ref()?
                    .iter()
                    .filter_map(|e| e.parse().ok())
                    .collect();
                (!levels.is_empty()).then(|| (c.model.clone(), levels))
            })
            .collect();
        let spec = ProviderSpec {
            custom: agenium_llm::custom::Custom {
                headers,
                efforts,
                // Checked when saved; a field edited into the file that breaks the rules is left out.
                body: pc
                    .body
                    .iter()
                    .filter_map(|f| {
                        agenium_llm::custom::check_field(&f.path, &f.value, f.model.as_deref()).ok()
                    })
                    .collect(),
                effort_field: pc
                    .effort_field
                    .as_deref()
                    .and_then(|f| agenium_llm::custom::check_path(f).ok()),
            },
            id: pc.id.clone(),
            kind: kind.into(),
            base_url: pc.base_url.clone(),
            api_key: key,
            max_concurrency: pc.max_concurrency as usize,
        };
        let p =
            agenium_llm::build_provider(&spec).map_err(|e| OrchError::Invalid(e.to_string()))?;
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_owned(), p.clone());
        Ok(p)
    }

    fn forget(&self, id: &str) {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }
}

/// Fixed providers by id (tests, evals).
pub struct StaticProviders(pub Vec<(String, Arc<dyn Provider>)>);

impl ProviderSource for StaticProviders {
    fn provider(&self, id: &str) -> Result<Arc<dyn Provider>> {
        self.0
            .iter()
            .find(|(k, _)| k == id)
            .map(|(_, p)| p.clone())
            .ok_or_else(|| OrchError::NotFound(format!("provider {id}")))
    }
}
