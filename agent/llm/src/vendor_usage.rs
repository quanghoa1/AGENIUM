//! Subscription usage of the vendors behind the agents (display only, decision 3k): how much of each rate-limit
//! window is used and when it resets. Sources are the vendors' own honest channels: the events a CLI reports during
//! a run (Claude's `rate_limit_event`), a CLI's own RPC (`codex app-server` `account/rateLimits/read`, see
//! `agenium_cli_backend::codex`), or a documented endpoint with the key Agenium already holds (OpenCode Go
//! `GET <base>/usage`). Nothing poses as another client.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::Http;
use crate::LlmError;

/// One rate-limit window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageWindow {
    /// `5h`, `day`, `week` or `month`.
    pub window: String,
    /// 0-100.
    pub used_percent: f64,
    /// RFC 3339.
    #[serde(default)]
    pub resets_at: Option<String>,
}

/// What a vendor last reported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VendorUsage {
    /// `claude`, `codex`, `agy`, or `opencode-go:<provider id>`.
    pub vendor: String,
    #[serde(default)]
    pub plan: Option<String>,
    #[serde(default)]
    pub windows: Vec<UsageWindow>,
    /// Extra facts worth a line (reset credits, a limit reached).
    #[serde(default)]
    pub note: Option<String>,
    /// The last attempt failed (earlier windows are kept and shown as stale).
    #[serde(default)]
    pub error: Option<String>,
    /// When the windows were read (RFC 3339); empty = never.
    #[serde(default)]
    pub updated_at: String,
}

impl VendorUsage {
    pub fn new(vendor: &str, now: &str) -> Self {
        Self {
            vendor: vendor.into(),
            plan: None,
            windows: vec![],
            note: None,
            error: None,
            updated_at: now.into(),
        }
    }
}

/// Unix seconds (or milliseconds) to RFC 3339.
fn epoch(v: &Value) -> Option<String> {
    let n = v.as_f64()?;
    let secs = if n > 1e11 { n / 1000.0 } else { n };
    chrono::DateTime::from_timestamp(secs as i64, 0).map(|d| d.to_rfc3339())
}

/// Claude Code's stream-json `rate_limit_event` (`rate_limit_info.unifiedWindows.{five_hour,seven_day}` with
/// `utilization` 0-1 and `resetsAt` in seconds).
pub fn from_claude_event(v: &Value, now: &str) -> Option<VendorUsage> {
    let info = &v["rate_limit_info"];
    if !info.is_object() {
        return None;
    }
    let mut u = VendorUsage::new("claude", now);
    for (key, window) in [("five_hour", "5h"), ("seven_day", "week")] {
        let w = &info["unifiedWindows"][key];
        if let Some(f) = w["utilization"].as_f64() {
            u.windows.push(UsageWindow {
                window: window.into(),
                used_percent: (f * 100.0).clamp(0.0, 100.0),
                resets_at: epoch(&w["resetsAt"]),
            });
        }
    }
    if info["status"] == "rejected" {
        u.note = Some(format!(
            "limit reached{}",
            epoch(&info["resetsAt"])
                .map(|t| format!(" until {t}"))
                .unwrap_or_default()
        ));
    }
    Some(u)
}

/// The result of `codex app-server` `account/rateLimits/read`: `rateLimits.{primary,secondary}` with `usedPercent`,
/// `windowDurationMins`, `resetsAt` (seconds), `planType`, and `rateLimitResetCredits.availableCount`.
pub fn from_codex_rate_limits(r: &Value, now: &str) -> VendorUsage {
    let mut u = VendorUsage::new("codex", now);
    let rl = &r["rateLimits"];
    u.plan = rl["planType"].as_str().map(str::to_owned);
    for (i, key) in ["primary", "secondary"].iter().enumerate() {
        let w = &rl[key];
        let Some(p) = w["usedPercent"].as_f64() else {
            continue;
        };
        let window = match w["windowDurationMins"].as_u64() {
            Some(m) if (299..=301).contains(&m) => "5h",
            Some(m) if (1439..=1441).contains(&m) => "day",
            Some(m) if (10079..=10081).contains(&m) => "week",
            _ if i == 0 => "5h",
            _ => "week",
        };
        u.windows.push(UsageWindow {
            window: window.into(),
            used_percent: p.clamp(0.0, 100.0),
            resets_at: epoch(&w["resetsAt"]),
        });
    }
    let mut notes = vec![];
    if let Some(t) = rl["rateLimitReachedType"].as_str() {
        notes.push(format!("limit reached ({t})"));
    }
    if let Some(n) = r["rateLimitResetCredits"]["availableCount"]
        .as_u64()
        .filter(|n| *n > 0)
    {
        notes.push(format!("{n} reset credit(s)"));
    }
    u.note = (!notes.is_empty()).then(|| notes.join(" · "));
    u
}

/// OpenCode Go `GET <base>/usage`: `usage.{rolling,weekly,monthly}` with `percent` (used), `resetsAt` (RFC 3339)
/// and `status` (`ok` or `rate-limited`).
pub fn from_opencode_go(provider: &str, v: &Value, now: &str) -> VendorUsage {
    let mut u = VendorUsage::new(&format!("opencode-go:{provider}"), now);
    u.plan = Some("Go".into());
    let mut limited = vec![];
    for (key, window) in [("rolling", "5h"), ("weekly", "week"), ("monthly", "month")] {
        let w = &v["usage"][key];
        let Some(p) = w["percent"].as_f64() else {
            continue;
        };
        if w["status"] == "rate-limited" {
            limited.push(window);
        }
        u.windows.push(UsageWindow {
            window: window.into(),
            used_percent: p.clamp(0.0, 100.0),
            resets_at: w["resetsAt"].as_str().map(str::to_owned),
        });
    }
    if !limited.is_empty() {
        u.note = Some(format!("limit reached ({})", limited.join(", ")));
    }
    u
}

/// Reads OpenCode Go usage with the provider's own key (`base_url` is the Go endpoint, e.g.
/// `https://opencode.ai/zen/go/v1`).
pub async fn fetch_opencode_go(
    provider: &str,
    base_url: &str,
    key: &str,
    now: &str,
) -> Result<VendorUsage, LlmError> {
    fn map_err(status: u16, _: &reqwest::header::HeaderMap, body: &str) -> LlmError {
        let why = match status {
            401 => "the key was refused".to_owned(),
            403 => "no OpenCode Go subscription on this key".to_owned(),
            _ => format!("HTTP {status}"),
        };
        LlmError::Network(format!(
            "{why}: {}",
            body.chars().take(160).collect::<String>()
        ))
    }
    let url = format!("{}/usage", base_url.trim_end_matches('/'));
    let v = Http::default_client()?
        .get_json(
            &url,
            &[
                ("authorization", format!("Bearer {key}")),
                ("accept", "application/json".into()),
            ],
            map_err,
        )
        .await?;
    if v["usage"].is_null() {
        return Err(LlmError::Decode(format!(
            "{url} returned no usage: {}",
            v.to_string().chars().take(160).collect::<String>()
        )));
    }
    Ok(from_opencode_go(provider, &v, now))
}

/// Whether a configured provider is the OpenCode Go relay (its usage is readable with the same key).
pub fn is_opencode_go(base_url: &str) -> bool {
    let b = base_url.to_ascii_lowercase();
    b.contains("opencode.ai/") && b.contains("/go/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vendor_reports_become_windows() {
        let c = from_claude_event(
            &json!({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "resetsAt": 1792903200,
                "unifiedWindows": {"five_hour": {"utilization": 0.09, "resetsAt": 1792903200},
                                   "seven_day": {"utilization": 0.18, "resetsAt": 1793224800}}}}),
            "now",
        )
        .unwrap();
        assert_eq!(c.windows.len(), 2);
        assert_eq!(c.windows[0].window, "5h");
        assert!((c.windows[1].used_percent - 18.0).abs() < 1e-9);
        assert!(c.windows[0]
            .resets_at
            .as_deref()
            .unwrap()
            .starts_with("2026-"));
        assert!(c.note.is_none());

        let x = from_codex_rate_limits(
            &json!({"rateLimits": {"planType": "plus",
                "primary": {"usedPercent": 12, "windowDurationMins": 300, "resetsAt": 1792918814},
                "secondary": {"usedPercent": 40, "windowDurationMins": 10080, "resetsAt": 1793425006}},
                "rateLimitResetCredits": {"availableCount": 2}}),
            "now",
        );
        assert_eq!(x.plan.as_deref(), Some("plus"));
        assert_eq!(
            x.windows
                .iter()
                .map(|w| w.window.as_str())
                .collect::<Vec<_>>(),
            ["5h", "week"]
        );
        assert_eq!(x.note.as_deref(), Some("2 reset credit(s)"));

        let o = from_opencode_go(
            "go",
            &json!({"usage": {"rolling": {"status": "ok", "percent": 5, "resetsAt": "2026-10-25T12:00:00Z"},
                              "weekly": {"status": "rate-limited", "percent": 100, "resetsAt": "2026-10-28T00:00:00Z"}}}),
            "now",
        );
        assert_eq!(o.vendor, "opencode-go:go");
        assert_eq!(o.windows.len(), 2);
        assert_eq!(o.note.as_deref(), Some("limit reached (week)"));
        assert!(is_opencode_go("https://opencode.ai/zen/go/v1"));
        assert!(!is_opencode_go("https://opencode.ai/zen/v1"));
    }
}
