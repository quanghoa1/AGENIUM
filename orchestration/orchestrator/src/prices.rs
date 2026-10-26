//! Model prices (display only, decision 3k): `<root>/pricing.toml`, the built-in table and the public catalogue
//! cached in `<root>/price-catalogue.json`. A run of an API model is priced when it ends; runs priced at 0 because
//! their model had no price yet are priced again when the catalogue is refreshed.

use std::collections::BTreeMap;

use agenium_llm::pricing::{fetch_catalogue, Catalogue, Price, PriceTable, Prices};
use agenium_llm::Usage;
use agenium_store::core::runs;
use serde::Serialize;

use crate::app::App;
use crate::{OrchError, Result};

/// The catalogue is refreshed when older than this.
pub const CATALOGUE_MAX_AGE_DAYS: i64 = 7;

/// What the app knows about prices, for the Usage page and the model pickers.
#[derive(Debug, Clone, Serialize)]
pub struct PriceView {
    /// Where the catalogue came from and when (`None`: never downloaded).
    pub source: Option<String>,
    pub fetched_at: Option<String>,
    /// Per configured provider: the price of each model it has one for (USD per million tokens).
    pub providers: BTreeMap<String, BTreeMap<String, Price>>,
}

/// The modification times of `pricing.toml` and the catalogue: the price list is read again when one changes.
pub(crate) type PricesStamp = (Option<std::time::SystemTime>, Option<std::time::SystemTime>);

impl App {
    /// The price list, read from disk only when `pricing.toml` or the catalogue changed since the last call.
    pub fn prices(&self) -> Prices {
        let layout = self.layout();
        let modified = |p: std::path::PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let stamp = (
            modified(layout.pricing()),
            modified(layout.price_catalogue()),
        );
        let mut cache = self.prices_cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, p)) = cache.as_ref() {
            if *at == stamp {
                return p.clone();
            }
        }
        let p = Prices {
            table: PriceTable::load(&layout.pricing()).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "pricing.toml unreadable; built-in prices only");
                PriceTable::builtin()
            }),
            catalogue: std::fs::read_to_string(layout.price_catalogue())
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
        };
        *cache = Some((stamp, p.clone()));
        p
    }

    /// Cost in USD of `usage` on the configured API provider `provider`; `None` when the model has no price or
    /// the provider is not an API provider (vendor CLIs run on subscriptions).
    pub fn usage_cost(
        &self,
        prices: &Prices,
        provider: &str,
        model: &str,
        u: &agenium_core::UsageSummary,
    ) -> Option<f64> {
        let cfg = self.config().provider(provider)?.clone();
        let kind = serde_json::to_value(cfg.kind).ok()?;
        prices.cost(
            &cfg.id,
            kind.as_str()?,
            cfg.base_url.as_deref(),
            model,
            &Usage {
                input: u.tokens_in,
                output: u.tokens_out,
                cache_read: u.cache_read,
                cache_write: u.cache_write,
                reasoning: 0,
            },
        )
    }

    pub fn price_view(&self) -> PriceView {
        let prices = self.prices();
        let mut providers = BTreeMap::new();
        for cfg in self.config().providers {
            let mut models: BTreeMap<String, Price> = prices
                .catalogue
                .provider(&cfg.id, cfg.base_url.as_deref())
                .map(|p| p.models.clone())
                .unwrap_or_default();
            let kind = serde_json::to_value(cfg.kind).unwrap_or_default();
            // Exact entries of pricing.toml and the built-in table win over the catalogue.
            for e in &prices.table.entries {
                if !e.model.ends_with('*')
                    && e.provider
                        .as_deref()
                        .is_none_or(|p| p == cfg.id || Some(p) == kind.as_str())
                {
                    if let Some(p) =
                        prices.lookup(&cfg.id, kind.as_str().unwrap_or(""), None, &e.model)
                    {
                        models.insert(e.model.clone(), p);
                    }
                }
            }
            providers.insert(cfg.id.clone(), models);
        }
        let known = !prices.catalogue.fetched_at.is_empty();
        PriceView {
            source: known.then(|| prices.catalogue.source.clone()),
            fetched_at: known.then(|| prices.catalogue.fetched_at.clone()),
            providers,
        }
    }

    /// OpenCode's free models still served (`opencode` in the catalogue, price 0, not deprecated): what the
    /// opencode CLI backend can run (ADR-68).
    pub fn opencode_free_models(&self) -> Vec<String> {
        self.prices()
            .catalogue
            .providers
            .iter()
            .find(|p| p.id == "opencode")
            .map(|p| {
                p.models
                    .iter()
                    .filter(|(m, price)| price.is_free() && !p.deprecated.contains(*m))
                    .map(|(m, _)| m.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// True when the catalogue was never downloaded or is older than [`CATALOGUE_MAX_AGE_DAYS`].
    pub fn price_catalogue_stale(&self) -> bool {
        let c = self.prices().catalogue;
        chrono::DateTime::parse_from_rfc3339(&c.fetched_at).map_or(true, |at| {
            chrono::Utc::now().signed_duration_since(at)
                > chrono::Duration::days(CATALOGUE_MAX_AGE_DAYS)
        })
    }

    /// Stores a downloaded catalogue and prices the runs that cost 0 for want of a price. Returns how many runs
    /// were priced.
    pub fn save_price_catalogue(&self, c: &Catalogue) -> Result<usize> {
        let path = self.layout().price_catalogue();
        let text = serde_json::to_string(c).map_err(|e| OrchError::Invalid(e.to_string()))?;
        agenium_config::write_atomic(&path, text.as_bytes())?;
        self.price_unpriced_runs()
    }

    /// Prices the finished API runs that have tokens and a cost of 0.
    pub fn price_unpriced_runs(&self) -> Result<usize> {
        let prices = self.prices();
        let todo: Vec<(agenium_core::RunId, agenium_core::UsageSummary)> = self
            .store
            .read(runs::unpriced)?
            .into_iter()
            .filter_map(|r| {
                let cost = self.usage_cost(&prices, &r.provider, &r.model, &r.usage)?;
                (cost > 0.0).then(|| {
                    let mut u = r.usage;
                    u.estimated_cost_usd = cost;
                    (r.id, u)
                })
            })
            .collect();
        if !todo.is_empty() {
            self.store.commit(None, "system", |w| -> Result<()> {
                for (id, u) in &todo {
                    runs::set_usage(w.tx, *id, u)?;
                }
                Ok(())
            })?;
        }
        Ok(todo.len())
    }
}

/// Downloads the catalogue named in the config and stores it. Returns the number of runs priced; `Ok(None)` when
/// the catalogue is turned off (`[usage] price_catalogue_url = ""`).
pub async fn refresh_price_catalogue(app: &App) -> Result<Option<usize>> {
    let url = app.config().usage.price_catalogue_url;
    if url.trim().is_empty() {
        return Ok(None);
    }
    let c = fetch_catalogue(&url, &agenium_core::now_iso())
        .await
        .map_err(|e| OrchError::Invalid(format!("price catalogue {url}: {e}")))?;
    app.save_price_catalogue(&c).map(Some)
}
