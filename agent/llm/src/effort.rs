//! The reasoning-effort levels of each model, by the vendor's own names (ADR-114). A vendor that reports them is
//! believed first (Anthropic's Models API `capabilities.effort`, OpenRouter's `reasoning.supported_efforts`, Codex's
//! `model/list`). This table covers the vendors whose model lists do not say (OpenAI, Gemini, xAI, DeepSeek), per
//! model, from their documentation (read 2026-10-26). A model the table does not know gets no effort: its vendor's
//! default.

use agenium_core::Effort;

use crate::ModelInfo;

/// A model's effort levels, lowest first, and the level its vendor uses when none is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Levels {
    pub levels: Vec<Effort>,
    pub default: Option<Effort>,
}

fn levels(names: &[&'static str], default: Option<&'static str>) -> Levels {
    Levels {
        levels: names
            .iter()
            .map(|n| n.parse().unwrap_or_default())
            .collect(),
        default: default.map(|d| d.parse().unwrap_or_default()),
    }
}

/// `m` is model `p` itself, a dated snapshot of it (`gpt-5.5-2026-04-01`) or its preview / latest alias; not a
/// sibling (`gpt-5.5-pro` is not `gpt-5.5`).
fn is(m: &str, p: &str) -> bool {
    let Some(rest) = m.strip_prefix(p) else {
        return false;
    };
    rest.is_empty()
        || rest.starts_with("-preview")
        || rest.starts_with("-latest")
        || rest
            .strip_prefix('-')
            .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
}

/// The model a Claude Code alias names now (`haiku`, `sonnet`, `opus`: the newest of each family).
pub fn alias_of(model: &str) -> Option<&'static str> {
    match model {
        "haiku" => Some("claude-haiku-5-5"),
        "sonnet" => Some("claude-sonnet-5-5"),
        "opus" => Some("claude-opus-5-5"),
        _ => None,
    }
}

/// The levels of `model` when this table knows it (`model` may carry a router's vendor prefix).
pub fn known(model: &str) -> Option<Levels> {
    let m = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    // Claude Code's aliases name the current models.
    let m = alias_of(&m).map_or(m, str::to_owned);
    if m.starts_with("claude-") {
        return Some(claude(&m.replace('.', "-")));
    }
    if m.starts_with("gpt-") {
        return openai(&m);
    }
    if m.starts_with("gemini-") {
        return gemini(&m);
    }
    if m.starts_with("grok-") {
        return xai(&m);
    }
    deepseek(&m)
}

/// Claude, as Anthropic's Models API reported it (`capabilities.effort`). Models without an effort parameter think
/// within a token budget; their levels are Agenium's budgets (`anthropic::budget`).
fn claude(m: &str) -> Levels {
    let budget_models = [
        "claude-3",
        "claude-haiku-4-5",
        "claude-sonnet-4-5",
        "claude-opus-4-1",
        "claude-opus-4-0",
        "claude-sonnet-4-0",
        "claude-opus-4-2",
        "claude-sonnet-4-2",
    ];
    if budget_models.iter().any(|p| m.starts_with(p)) {
        return levels(&["low", "medium", "high", "max"], Some("medium"));
    }
    if m.starts_with("claude-opus-4-5") {
        return levels(&["low", "medium", "high"], Some("high"));
    }
    if m.starts_with("claude-opus-4-6") || m.starts_with("claude-sonnet-4-6") {
        return levels(&["low", "medium", "high", "max"], Some("high"));
    }
    let default = if m.starts_with("claude-haiku-5-5") || m.starts_with("claude-opus-5-5") {
        "medium"
    } else {
        "high"
    };
    levels(&["low", "medium", "high", "xhigh", "max"], Some(default))
}

/// OpenAI (`reasoning_effort` / `reasoning.effort`), from each model's page.
fn openai(m: &str) -> Option<Levels> {
    const SIX: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
    const SIX_NONE: [&str; 6] = ["none", "low", "medium", "high", "xhigh", "max"];
    const FIVE_NONE: [&str; 5] = ["none", "low", "medium", "high", "xhigh"];
    let table: [(&str, &[&'static str], Option<&'static str>); 14] = [
        ("gpt-6-astra", &SIX, None),
        ("gpt-6.1-sol", &SIX, Some("medium")),
        ("gpt-6-sol", &SIX_NONE, Some("medium")),
        ("gpt-6-luna", &SIX_NONE, Some("medium")),
        ("gpt-5.6-sol", &SIX_NONE, Some("medium")),
        ("gpt-5.6-terra", &SIX_NONE, Some("medium")),
        ("gpt-5.6-luna", &SIX_NONE, Some("medium")),
        ("gpt-5.5-pro", &["medium", "high", "xhigh"], Some("high")),
        ("gpt-5.5", &FIVE_NONE, Some("medium")),
        ("gpt-5.4-mini", &FIVE_NONE, Some("none")),
        ("gpt-5.4", &FIVE_NONE, Some("none")),
        ("gpt-5.2", &FIVE_NONE, Some("none")),
        ("gpt-5.1", &["none", "low", "medium", "high"], Some("none")),
        ("gpt-5", &["minimal", "low", "medium", "high"], None),
    ];
    table
        .iter()
        .find(|(p, ..)| is(m, p))
        .map(|(_, l, d)| levels(l, *d))
}

/// Gemini (`thinkingConfig.thinkingLevel`), from each model's page; 2.5 thinks within a budget (Agenium's budgets,
/// `gemini::thinking_config`).
fn gemini(m: &str) -> Option<Levels> {
    const LMH: [&str; 3] = ["low", "medium", "high"];
    const MLMH: [&str; 4] = ["minimal", "low", "medium", "high"];
    let table: [(&str, &[&'static str], Option<&'static str>); 10] = [
        ("gemini-4-argon", &LMH, Some("medium")),
        ("gemini-3.8-flash", &LMH, Some("medium")),
        ("gemini-3.7-flash", &LMH, Some("medium")),
        ("gemini-3.6-flash", &MLMH, Some("medium")),
        ("gemini-3.5-flash-lite", &MLMH, Some("minimal")),
        ("gemini-3.1-pro", &LMH, Some("high")),
        ("gemini-3.1-flash-lite", &MLMH, Some("minimal")),
        ("gemini-3-flash", &MLMH, Some("high")),
        ("gemini-3-pro", &["low", "high"], Some("high")),
        ("gemini-2.5", &["low", "medium", "high", "max"], None),
    ];
    table
        .iter()
        .find(|(p, ..)| {
            if p.ends_with("2.5") {
                m.starts_with(p)
            } else {
                is(m, p)
            }
        })
        .map(|(_, l, d)| levels(l, *d))
}

/// xAI (`reasoning_effort`), from each model's page.
fn xai(m: &str) -> Option<Levels> {
    const LMHX: [&str; 4] = ["low", "medium", "high", "xhigh"];
    let table: [(&str, &[&'static str], &'static str); 4] = [
        ("grok-4.7", &LMHX, "high"),
        ("grok-4.6", &LMHX, "high"),
        ("grok-4.5", &LMHX, "high"),
        (
            "grok-4.3",
            &["none", "low", "medium", "high", "xhigh"],
            "low",
        ),
    ];
    table
        .iter()
        .find(|(p, ..)| is(m, p))
        .map(|(_, l, d)| levels(l, Some(d)))
}

/// DeepSeek V4 (`reasoning_effort`): three levels; `medium` is not one of them (DeepSeek maps it to high).
fn deepseek(m: &str) -> Option<Levels> {
    let retired = ["deepseek-chat", "deepseek-reasoner", "deepseek-coder"];
    (m.starts_with("deepseek-") && !retired.iter().any(|r| m.starts_with(r)))
        .then(|| levels(&["low", "high", "max"], Some("high")))
}

/// The level to send for `model`: the chosen one resolved to the model's own levels. `None` when the model has no
/// effort control or this table does not know it (the vendor's default then applies).
pub fn level_for(model: &str, chosen: &Effort) -> Option<Effort> {
    known(model).and_then(|l| chosen.resolve(&l.levels))
}

/// Fills what a listing did not report from this table.
pub fn fill(info: &mut ModelInfo) {
    if let Some(l) = known(&info.id) {
        if info.efforts.is_none() {
            info.efforts = Some(l.levels);
        }
        if info.default_effort.is_none() {
            info.default_effort = l.default;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(m: &str) -> String {
        known(m)
            .map(|l| {
                l.levels
                    .iter()
                    .map(|e| e.as_str().to_owned())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_else(|| "?".into())
    }

    #[test]
    fn claude_levels_follow_the_models_api() {
        assert_eq!(names("claude-haiku-5-5"), "low,medium,high,xhigh,max");
        assert_eq!(names("haiku"), "low,medium,high,xhigh,max");
        assert_eq!(names("anthropic/claude-sonnet-4.6"), "low,medium,high,max");
        assert_eq!(names("claude-opus-4-5-20251101"), "low,medium,high");
        assert_eq!(
            known("claude-sonnet-5-5").unwrap().default,
            Some(Effort::HIGH)
        );
        assert_eq!(
            known("claude-opus-5-5").unwrap().default,
            Some(Effort::MEDIUM)
        );
        assert_eq!(
            level_for("claude-sonnet-4-6", &Effort::XHIGH),
            Some(Effort::MAX)
        );
    }

    #[test]
    fn each_model_has_its_own_levels() {
        assert_eq!(names("deepseek-v4.1-flash"), "low,high,max");
        assert_eq!(
            level_for("deepseek-v4-pro", &Effort::MEDIUM),
            Some(Effort::HIGH)
        );
        assert_eq!(names("gemini-3-pro-preview"), "low,high");
        assert_eq!(names("gemini-3.8-flash"), "low,medium,high");
        assert_eq!(names("gemini-3-flash-preview"), "minimal,low,medium,high");
        assert_eq!(names("gpt-5.5-pro"), "medium,high,xhigh");
        assert_eq!(names("gpt-5.5"), "none,low,medium,high,xhigh");
        assert_eq!(names("gpt-5.5-2026-04-01"), "none,low,medium,high,xhigh");
        assert_eq!(names("gpt-5"), "minimal,low,medium,high");
        assert_eq!(names("grok-4.7"), "low,medium,high,xhigh");
        assert_eq!(level_for("grok-4.7", &Effort::MAX), Some(Effort::XHIGH));
        assert_eq!(
            names("gpt-5-mini"),
            "?",
            "not documented: the vendor's default"
        );
        assert_eq!(names("grok-4.7-build-fast"), "?");
        assert_eq!(names("llama-4-scout"), "?");
        assert_eq!(level_for("llama-4-scout", &Effort::HIGH), None);
    }
}
