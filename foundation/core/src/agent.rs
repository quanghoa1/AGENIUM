//! Agents: long-lived registry entities (no subagents). The model is a swappable binding.

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

/// Reasoning effort chosen by the user; adapters map it per vendor (no separate thinking switch, decision 13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    #[default]
    Medium,
    High,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }
}

impl FromStr for Effort {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "max" => Ok(Self::Max),
            _ => Err(format!("unknown effort {s:?} (low|medium|high|max)")),
        }
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
        assert_eq!(serde_json::to_string(&Effort::Max).unwrap(), "\"max\"");
        assert_eq!("high".parse::<Effort>().unwrap(), Effort::High);
        assert!(!AgentStatus::Offline.assignable() && AgentStatus::Waiting.assignable());
    }
}
