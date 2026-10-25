//! Display-only price table (decision 3k: cost is a metric, never a limit). Ships with built-in Anthropic list
//! prices (USD per million tokens, from the Claude API model table cached 2026-10-06) and is overridden/extended by
//! `<root>/pricing.toml`. Other API models take their price from a public model catalogue (models.dev format),
//! downloaded into `<root>/price-catalogue.json`. Unknown models cost "unknown", not zero; local endpoints cost 0
//! (decision 31).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::types::Usage;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceEntry {
    #[serde(default)]
    pub provider: Option<String>,
    /// Exact model id, or a prefix ending in `*`.
    pub model: String,
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
}

impl PriceEntry {
    fn price(&self) -> Price {
        Price {
            input: self.input,
            output: self.output,
            // Typical vendor ratios when not given: cache reads 0.1x, 5-minute cache writes 1.25x input.
            cache_read: self.cache_read.unwrap_or(self.input * 0.1),
            cache_write: self.cache_write.unwrap_or(self.input * 1.25),
        }
    }

    fn matches(&self, provider: &str, model: &str) -> bool {
        if self.provider.as_deref().is_some_and(|p| p != provider) {
            return false;
        }
        match self.model.strip_suffix('*') {
            Some(prefix) => model.starts_with(prefix),
            None => model == self.model,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PriceTable {
    #[serde(default, rename = "model")]
    pub entries: Vec<PriceEntry>,
}

fn e(model: &str, input: f64, output: f64, cache_read: Option<f64>) -> PriceEntry {
    PriceEntry {
        provider: Some("anthropic".into()),
        model: model.into(),
        input,
        output,
        cache_read,
        cache_write: None,
    }
}

impl PriceTable {
    pub fn builtin() -> Self {
        Self {
            entries: vec![
                e("claude-fable-5-1", 10.0, 50.0, Some(0.25)),
                e("claude-fable-5", 10.0, 50.0, None),
                e("claude-opus-5-5", 4.0, 20.0, Some(0.20)),
                e("claude-opus-5", 5.0, 25.0, None),
                e("claude-opus-4-8", 5.0, 25.0, None),
                e("claude-opus-4-7", 5.0, 25.0, None),
                e("claude-opus-4-6", 5.0, 25.0, None),
                e("claude-sonnet-5-5", 2.0, 10.0, Some(0.20)),
                e("claude-sonnet-5", 2.0, 10.0, None),
                e("claude-sonnet-4-6", 3.0, 15.0, None),
                e("claude-haiku-5-5", 0.10, 0.50, None),
                e("claude-haiku-4-5", 1.0, 5.0, None),
            ],
        }
    }

    /// Built-in table with `<root>/pricing.toml` entries taking precedence.
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut t = Self::builtin();
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let user: PriceTable =
                    toml::from_str(&s).map_err(|e| format!("{}: {e}", path.display()))?;
                let mut entries = user.entries;
                entries.extend(t.entries);
                t.entries = entries;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
        Ok(t)
    }

    pub fn lookup(&self, provider_kind: &str, model: &str) -> Option<Price> {
        self.entries
            .iter()
            .find(|e| e.matches(provider_kind, model))
            .map(PriceEntry::price)
    }

    /// Estimated cost in USD, `None` when the model has no price.
    pub fn cost(&self, provider_kind: &str, model: &str, u: &Usage) -> Option<f64> {
        self.lookup(provider_kind, model).map(|p| p.cost(u))
    }
}

/// The prices of one provider of the catalogue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogueProvider {
    pub id: String,
    /// The provider's API base URL, used to match a configured provider whose id differs.
    #[serde(default)]
    pub api: Option<String>,
    pub models: BTreeMap<String, Price>,
    /// Models the provider no longer serves (`status: deprecated`); their prices still price earlier runs.
    #[serde(default)]
    pub deprecated: std::collections::BTreeSet<String>,
}

/// Model prices from a public catalogue, trimmed to what pricing needs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalogue {
    pub fetched_at: String,
    pub source: String,
    pub providers: Vec<CatalogueProvider>,
}

impl Catalogue {
    /// Reads the models.dev format: `{ "<provider>": { "api": .., "models": { "<model>": { "cost": { "input": ..,
    /// "output": .., "cache_read": .., "cache_write": .. } } } } }`, USD per million tokens. Models without a cost
    /// are left out.
    pub fn from_models_dev(v: &serde_json::Value, source: &str, fetched_at: &str) -> Self {
        let mut providers = Vec::new();
        for (id, p) in v.as_object().into_iter().flatten() {
            let mut models = BTreeMap::new();
            let mut deprecated = std::collections::BTreeSet::new();
            for (mid, m) in p["models"].as_object().into_iter().flatten() {
                let c = &m["cost"];
                let (Some(input), Some(output)) = (c["input"].as_f64(), c["output"].as_f64())
                else {
                    continue;
                };
                let entry = PriceEntry {
                    provider: None,
                    model: mid.clone(),
                    input,
                    output,
                    // No cache price listed: the provider bills cached tokens as plain input.
                    cache_read: c["cache_read"].as_f64().or(Some(input)),
                    cache_write: c["cache_write"].as_f64().or(Some(input)),
                };
                models.insert(mid.clone(), entry.price());
                if m["status"] == "deprecated" {
                    deprecated.insert(mid.clone());
                }
            }
            if !models.is_empty() {
                providers.push(CatalogueProvider {
                    id: id.clone(),
                    api: p["api"].as_str().map(str::to_owned),
                    models,
                    deprecated,
                });
            }
        }
        Self {
            fetched_at: fetched_at.into(),
            source: source.into(),
            providers,
        }
    }

    /// The catalogue provider of a configured provider: the same id, else the one whose API URL is on the same
    /// host and shares the most path segments with `base_url` (a tie on a host means no match).
    pub fn provider(&self, id: &str, base_url: Option<&str>) -> Option<&CatalogueProvider> {
        if let Some(p) = self.providers.iter().find(|p| p.id == id) {
            return Some(p);
        }
        let (host, path) = split_url(base_url?)?;
        let mut best: Vec<(usize, &CatalogueProvider)> = self
            .providers
            .iter()
            .filter_map(|p| {
                let (h, ps) = split_url(p.api.as_deref()?)?;
                (h == host).then(|| (ps.iter().filter(|s| path.contains(s)).count(), p))
            })
            .collect();
        best.sort_by_key(|b| std::cmp::Reverse(b.0));
        match best.as_slice() {
            [(_, p)] => Some(p),
            [(a, p), (b, _), ..] if a > b => Some(p),
            _ => None,
        }
    }
}

/// Downloads the catalogue at `url` (models.dev format).
pub async fn fetch_catalogue(url: &str, fetched_at: &str) -> Result<Catalogue, crate::LlmError> {
    fn map_err(status: u16, _: &reqwest::header::HeaderMap, body: &str) -> crate::LlmError {
        crate::LlmError::Network(format!(
            "HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        ))
    }
    let v = crate::http::Http::default_client()?
        .get_json(url, &[], map_err)
        .await?;
    Ok(Catalogue::from_models_dev(&v, url, fetched_at))
}

/// Host and the meaningful path segments of a URL (version and wire names left out).
fn split_url(url: &str) -> Option<(String, Vec<String>)> {
    let rest = url.split("://").nth(1)?;
    let mut parts = rest.split('/');
    let host = parts.next()?.to_ascii_lowercase();
    let segs = parts
        .filter(|s| !s.is_empty() && !matches!(*s, "v1" | "v2" | "api" | "openai" | "anthropic"))
        .map(str::to_ascii_lowercase)
        .collect();
    Some((host, segs))
}

/// The price list in use: `<root>/pricing.toml` and the built-in table first, then the catalogue.
#[derive(Debug, Clone, Default)]
pub struct Prices {
    pub table: PriceTable,
    pub catalogue: Catalogue,
}

impl Prices {
    /// Price of `model` on the configured provider `id` (of wire `kind`, at `base_url`).
    pub fn lookup(
        &self,
        id: &str,
        kind: &str,
        base_url: Option<&str>,
        model: &str,
    ) -> Option<Price> {
        self.table
            .entries
            .iter()
            .find(|e| e.matches(kind, model) || e.matches(id, model))
            .map(PriceEntry::price)
            .or_else(|| {
                self.catalogue
                    .provider(id, base_url)
                    .and_then(|p| p.models.get(model).copied())
            })
    }

    pub fn cost(
        &self,
        id: &str,
        kind: &str,
        base_url: Option<&str>,
        model: &str,
        u: &Usage,
    ) -> Option<f64> {
        self.lookup(id, kind, base_url, model).map(|p| p.cost(u))
    }
}

impl Price {
    /// Cost in USD of `u` at this price.
    pub fn cost(&self, u: &Usage) -> f64 {
        (u.input as f64 * self.input
            + u.output as f64 * self.output
            + u.cache_read as f64 * self.cache_read
            + u.cache_write as f64 * self.cache_write)
            / 1_000_000.0
    }

    pub fn is_free(&self) -> bool {
        self.input == 0.0 && self.output == 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_and_overrides() {
        let t = PriceTable::builtin();
        let u = Usage {
            input: 1_000_000,
            output: 100_000,
            cache_read: 1_000_000,
            cache_write: 0,
            reasoning: 0,
        };
        let c = t.cost("anthropic", "claude-opus-5-5", &u).unwrap();
        assert!((c - (4.0 + 2.0 + 0.2)).abs() < 1e-9, "{c}");
        assert!(
            t.cost("openai_compat", "gpt-x", &u).is_none(),
            "unknown is not zero"
        );
        let dir = std::env::temp_dir().join(format!("agenium-pricing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("pricing.toml");
        std::fs::write(&p, "[[model]]\nmodel = \"llama*\"\ninput = 0\noutput = 0\n[[model]]\nprovider = \"anthropic\"\nmodel = \"claude-opus-5-5\"\ninput = 1\noutput = 1\n").unwrap();
        let t = PriceTable::load(&p).unwrap();
        assert_eq!(t.cost("openai_compat", "llama3.2:8b", &u), Some(0.0));
        assert!(
            t.cost("anthropic", "claude-opus-5-5", &u).unwrap() < 2.0,
            "user entries win"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn catalogue_prices_api_models() {
        let v = serde_json::json!({
            "opencode-go": {"api": "https://opencode.ai/inference/go/openai/v1", "models": {
                "deepseek-v4.1-flash": {"cost": {"input": 0.15, "output": 0.6, "cache_read": 0.003}},
                "beam-free": {"cost": {"input": 0, "output": 0}},
                "old-free": {"cost": {"input": 0, "output": 0}, "status": "deprecated"},
                "no-price": {}
            }},
            "opencode": {"api": "https://opencode.ai/inference/openai/v1", "models": {
                "deepseek-v4.1-flash": {"cost": {"input": 0.3, "output": 1.2}}
            }}
        });
        let c = Catalogue::from_models_dev(&v, "test", "2026-10-25");
        assert!(!c
            .providers
            .iter()
            .any(|p| p.models.contains_key("no-price")));
        assert!(c
            .provider("opencode-go", None)
            .unwrap()
            .deprecated
            .contains("old-free"));
        // Matched by id, then by host and path ("go") when the configured id differs.
        assert_eq!(c.provider("opencode-go", None).unwrap().id, "opencode-go");
        assert_eq!(
            c.provider("my-go", Some("https://opencode.ai/zen/go/v1"))
                .unwrap()
                .id,
            "opencode-go"
        );
        assert!(c.provider("x", Some("https://example.com/v1")).is_none());
        let prices = Prices {
            table: PriceTable::builtin(),
            catalogue: c,
        };
        let u = Usage {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 0,
            reasoning: 0,
        };
        let cost = prices
            .cost(
                "my-go",
                "openai_compat",
                Some("https://opencode.ai/zen/go/v1"),
                "deepseek-v4.1-flash",
                &u,
            )
            .unwrap();
        assert!((cost - (0.15 + 0.6 + 0.003)).abs() < 1e-9, "{cost}");
        assert!(prices
            .lookup("opencode-go", "openai_compat", None, "beam-free")
            .unwrap()
            .is_free());
        assert!(prices
            .lookup("opencode-go", "openai_compat", None, "gpt-x")
            .is_none());
        // The built-in table still prices Anthropic models.
        assert!(prices
            .lookup("anthropic", "anthropic", None, "claude-opus-5-5")
            .is_some());
    }
}
