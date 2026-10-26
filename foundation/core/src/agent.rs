//! Agents: long-lived registry entities (no subagents). The model is a swappable binding.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Agent runtime status (decision 3i). QuotaExhausted and Offline belong to the agent, never to a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum AgentStatus {
    Idle,
    Working,
    /// Waiting for an answer/approval; takes no new work.
    Waiting,
    /// The provider reported exhausted quota/credit (402 or 429 with a quota code).
    QuotaExhausted,
    /// Retries exhausted; a background probe resumes the agent.
    Offline,
}

impl AgentStatus {
    pub const ALL: [AgentStatus; 5] = [
        Self::Idle,
        Self::Working,
        Self::Waiting,
        Self::QuotaExhausted,
        Self::Offline,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Working => "Working",
            Self::Waiting => "Waiting",
            Self::QuotaExhausted => "QuotaExhausted",
            Self::Offline => "Offline",
        }
    }

    /// Statuses that can receive an assignment from the Lead.
    pub fn assignable(self) -> bool {
        !matches!(self, Self::QuotaExhausted | Self::Offline)
    }
}

impl fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AgentStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|v| v.as_str() == s)
            .ok_or_else(|| format!("unknown agent status {s:?}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RoleKind {
    Lead,
    Worker,
}

/// Reasoning effort: one of the model's own levels, by the vendor's name (`low`, `high`, `max`, Codex's `ultra`, ...).
/// Each model has its own levels, reported by its vendor or known per model (ADR-114); the well-known names in
/// [`Effort::ORDER`] rank them when a level chosen for one model must be resolved for another.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Effort(Cow<'static, str>);

impl Default for Effort {
    fn default() -> Self {
        Self::MEDIUM
    }
}

impl Effort {
    /// Well-known level names, lowest first. Vendors use subsets: Claude low..max, OpenAI none..xhigh, DeepSeek
    /// high and max, Gemini minimal..high, Codex low..ultra.
    pub const ORDER: [&'static str; 8] = [
        "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
    ];
    pub const LOW: Self = Self(Cow::Borrowed("low"));
    pub const MEDIUM: Self = Self(Cow::Borrowed("medium"));
    pub const HIGH: Self = Self(Cow::Borrowed("high"));
    pub const XHIGH: Self = Self(Cow::Borrowed("xhigh"));
    pub const MAX: Self = Self(Cow::Borrowed("max"));

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Position among the well-known names; `None` for a name only some vendor uses.
    pub fn rank(&self) -> Option<usize> {
        Self::ORDER.iter().position(|n| *n == self.as_str())
    }

    /// This level for a model that takes `levels`: itself when the model has it, else the model's nearest level (on
    /// a tie the higher one, so a level is never quietly lowered: DeepSeek, without medium, gets high, as DeepSeek
    /// itself maps it). `None` when the model has no levels (send no effort).
    pub fn resolve(&self, levels: &[Effort]) -> Option<Effort> {
        if levels.contains(self) {
            return Some(self.clone());
        }
        let Some(r) = self.rank() else {
            return levels.first().cloned();
        };
        levels
            .iter()
            .filter_map(|l| l.rank().map(|x| (x.abs_diff(r), std::cmp::Reverse(x), l)))
            .min_by_key(|(d, x, _)| (*d, *x))
            .map(|(.., l)| l.clone())
            .or_else(|| levels.first().cloned())
    }
}

impl fmt::Display for Effort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Effort {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim().to_ascii_lowercase();
        if t.is_empty()
            || t.len() > 32
            || !t
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!("not an effort level: {s:?}"));
        }
        Ok(Self(Cow::Owned(t)))
    }
}

/// Which provider/model an agent uses. Changing it never touches memory, MCP or skills.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelBinding {
    /// Provider id from `config.toml` (`[[providers]] id`).
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub effort: Effort,
    /// Hard output cap per request; `None` = adapter default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
pub enum BackendKind {
    #[default]
    Api,
    Cli,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for s in AgentStatus::ALL {
            assert_eq!(s.as_str().parse::<AgentStatus>().unwrap(), s);
        }
        assert_eq!(serde_json::to_string(&Effort::MAX).unwrap(), "\"max\"");
        assert_eq!("High".parse::<Effort>().unwrap(), Effort::HIGH);
        assert_eq!(
            serde_json::from_str::<Effort>("\"ultra\"")
                .unwrap()
                .as_str(),
            "ultra"
        );
        assert!("x y".parse::<Effort>().is_err());
        assert!(!AgentStatus::Offline.assignable() && AgentStatus::Waiting.assignable());
    }

    #[test]
    fn a_level_resolves_to_the_models_own() {
        let l = |v: &[&str]| {
            v.iter()
                .map(|x| x.parse::<Effort>().unwrap())
                .collect::<Vec<_>>()
        };
        let deepseek = l(&["low", "high", "max"]);
        assert_eq!(
            Effort::MEDIUM.resolve(&deepseek),
            Some(Effort::HIGH),
            "a tie: the higher"
        );
        assert_eq!(Effort::MAX.resolve(&deepseek), Some(Effort::MAX));
        assert_eq!(
            "minimal".parse::<Effort>().unwrap().resolve(&deepseek),
            Some(Effort::LOW)
        );
        assert_eq!(
            Effort::MAX.resolve(&l(&["low", "medium", "high", "xhigh"])),
            Some(Effort::XHIGH)
        );
        let codex = l(&["low", "medium", "high", "xhigh", "max", "ultra"]);
        assert_eq!(
            "ultra"
                .parse::<Effort>()
                .unwrap()
                .resolve(&codex)
                .unwrap()
                .as_str(),
            "ultra"
        );
        assert_eq!(
            "ultra"
                .parse::<Effort>()
                .unwrap()
                .resolve(&l(&["low", "medium", "high"])),
            Some(Effort::HIGH)
        );
        assert_eq!(Effort::HIGH.resolve(&[]), None, "no levels: no effort sent");
    }
}
