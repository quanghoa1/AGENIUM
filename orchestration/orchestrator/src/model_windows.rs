//! A model's context window from the models.dev catalogue the root keeps (`models-dev.json`, ADR-153), for a model
//! its provider does not describe and for the agent CLIs (ADR-155). The provider's own list and a hand-set
//! `context_window` come first; this comes before the 200 000-token default, which made a run on a smaller model
//! compact too late and fail on the provider's refusal.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::app::App;

/// Context windows by catalogue provider and model id.
#[derive(Debug, Default)]
pub struct Windows {
    by_provider: BTreeMap<String, BTreeMap<String, u64>>,
}

/// The catalogue provider of an agent CLI's models.
fn cli_vendor(cli: &str) -> Option<&'static str> {
    Some(match cli {
        "claude" => "anthropic",
        "codex" => "openai",
        "agy" => "google",
        "grok" => "xai",
        _ => return None,
    })
}

impl Windows {
    /// Reads the models.dev format: `{ "<provider>": { "models": { "<model>": { "limit": { "context": .. } } } } }`.
    pub fn from_models_dev(v: &serde_json::Value) -> Self {
        let mut by_provider = BTreeMap::new();
        for (pid, p) in v.as_object().into_iter().flatten() {
            let models: BTreeMap<String, u64> = p["models"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(mid, m)| Some((mid.clone(), m["limit"]["context"].as_u64()?)))
                .filter(|(_, c)| *c > 0)
                .collect();
            if !models.is_empty() {
                by_provider.insert(pid.clone(), models);
            }
        }
        Self { by_provider }
    }

    /// The window of `model` served by `provider` (a configured provider's id, or an agent CLI's name). In order: the
    /// same provider and model; the CLI's vendor and the model, or a single family its alias names ("haiku"); the
    /// same model id at any provider (the smallest window listed: it must fit wherever it runs).
    pub fn get(&self, provider: &str, model: &str) -> Option<u64> {
        let (prefix, bare) = match model.split_once('/') {
            Some((p, m)) => (Some(p), m),
            None => (None, model),
        };
        for pid in [Some(provider), prefix, cli_vendor(provider)]
            .into_iter()
            .flatten()
        {
            if let Some(ms) = self.by_provider.get(pid) {
                if let Some(w) = ms.get(model).or_else(|| ms.get(bare)) {
                    return Some(*w);
                }
            }
        }
        if let Some(ms) = cli_vendor(provider).and_then(|v| self.by_provider.get(v)) {
            let alias = bare.to_lowercase();
            let family: Vec<u64> = ms
                .iter()
                .filter(|(id, _)| id.to_lowercase().contains(&alias))
                .map(|(_, w)| *w)
                .collect();
            if let Some(w) = family.iter().min() {
                return Some(*w);
            }
        }
        self.by_provider
            .values()
            .filter_map(|ms| ms.get(model).or_else(|| ms.get(bare)))
            .min()
            .copied()
    }
}

type Cached = Option<(PathBuf, Option<SystemTime>, Arc<Windows>)>;

static CACHE: Mutex<Cached> = Mutex::new(None);

/// The windows of the catalogue at `path`, read again when the file changes.
fn windows_at(path: &Path) -> Arc<Windows> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((p, m, w)) = cache.as_ref() {
        if p == path && *m == modified {
            return w.clone();
        }
    }
    let w = Arc::new(
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .map(|v| Windows::from_models_dev(&v))
            .unwrap_or_default(),
    );
    *cache = Some((path.to_path_buf(), modified, w.clone()));
    w
}

impl App {
    /// The context window the models.dev catalogue lists for `model` of `provider` (a provider id or a CLI's name).
    pub fn catalogue_window(&self, provider: &str, model: &str) -> Option<u64> {
        windows_at(&self.layout().models_dev()).get(provider, model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_is_found_by_provider_cli_alias_or_model_id() {
        let w = Windows::from_models_dev(&serde_json::json!({
            "anthropic": {"models": {
                "claude-haiku-4-5": {"limit": {"context": 200000}},
                "claude-sonnet-4-6": {"limit": {"context": 1000000}}}},
            "deepseek": {"models": {"deepseek-v4.1-flash": {"limit": {"context": 1000000}}}},
            "opencode-go": {"models": {"deepseek-v4.1-flash": {"limit": {"context": 256000}}}},
            "nolimit": {"models": {"x": {}}}
        }));
        assert_eq!(w.get("opencode-go", "deepseek-v4.1-flash"), Some(256_000));
        // Another provider's id: the smallest window listed for that model.
        assert_eq!(w.get("my-gateway", "deepseek-v4.1-flash"), Some(256_000));
        // A CLI alias names a family of its vendor.
        assert_eq!(w.get("claude", "haiku"), Some(200_000));
        assert_eq!(w.get("claude", "claude-sonnet-4-6"), Some(1_000_000));
        // opencode's provider/model form.
        assert_eq!(
            w.get("opencode", "deepseek/deepseek-v4.1-flash"),
            Some(1_000_000)
        );
        assert_eq!(w.get("x", "unknown"), None);
    }
}
