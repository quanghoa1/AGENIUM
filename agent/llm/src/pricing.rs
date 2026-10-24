//! Display-only price table (decision 3k: cost is a metric, never a limit). Ships with built-in Anthropic list
//! prices (USD per million tokens, from the Claude API model table cached 2026-10-06) and is overridden/extended by
//! `<root>/pricing.toml`. Unknown models cost "unknown", not zero; local endpoints cost 0 (decision 31).

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
        let p = self.lookup(provider_kind, model)?;
        Some(
            (u.input as f64 * p.input
                + u.output as f64 * p.output
                + u.cache_read as f64 * p.cache_read
                + u.cache_write as f64 * p.cache_write)
                / 1_000_000.0,
        )
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
}
