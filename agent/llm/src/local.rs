//! Capability detection for Local / custom OpenAI-compatible endpoints (ADR-31): tool calling, vision and context
//! length are not part of `/v1/models`, so the servers' own endpoints are asked, best effort:
//! LM Studio `GET /api/v0/models` (`capabilities: ["tool_use"]`, `type: "vlm"`, `max_context_length`), then Ollama
//! `POST /api/show` per model (`capabilities: ["tools", "vision"]`, `model_info.<arch>.context_length`).
//! Anything that does not answer leaves the flags unknown; manual flags in `config.toml` win over these.

use std::time::Duration;

use serde_json::{json, Value};

use crate::http::Http;
use crate::types::ModelInfo;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The server root of an OpenAI-compatible base URL (`http://localhost:11434/v1` -> `http://localhost:11434`).
fn root(base_url: &str) -> &str {
    let b = base_url.trim_end_matches('/');
    b.strip_suffix("/v1").unwrap_or(b)
}

async fn get(http: &Http, url: &str) -> Option<Value> {
    let r = http
        .client()
        .get(url)
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !r.status().is_success() {
        return None;
    }
    r.json().await.ok()
}

async fn post(http: &Http, url: &str, body: &Value) -> Option<Value> {
    let r = http
        .client()
        .post(url)
        .json(body)
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !r.status().is_success() {
        return None;
    }
    r.json().await.ok()
}

fn has(caps: &Value, name: &str) -> bool {
    caps.as_array()
        .is_some_and(|a| a.iter().any(|c| c.as_str() == Some(name)))
}

/// Fills the unknown capability flags of `models` from the server's own endpoints.
pub async fn enrich(http: &Http, base_url: &str, models: &mut [ModelInfo]) {
    let root = root(base_url);
    if let Some(v) = get(http, &format!("{root}/api/v0/models")).await {
        let list = v["data"].as_array().cloned().unwrap_or_default();
        for m in models.iter_mut() {
            if let Some(d) = list
                .iter()
                .find(|d| d["id"].as_str() == Some(m.id.as_str()))
            {
                m.supports_tools = m
                    .supports_tools
                    .or(Some(has(&d["capabilities"], "tool_use")));
                m.supports_vision = m.supports_vision.or(Some(d["type"] == "vlm"));
                m.context_window = m.context_window.or(d["max_context_length"].as_u64());
            }
        }
        return;
    }
    for m in models.iter_mut() {
        let Some(show) = post(http, &format!("{root}/api/show"), &json!({"model": m.id})).await
        else {
            // Not Ollama either: nothing more to learn from this server.
            return;
        };
        if show["capabilities"].is_array() {
            m.supports_tools = m
                .supports_tools
                .or(Some(has(&show["capabilities"], "tools")));
            m.supports_vision = m
                .supports_vision
                .or(Some(has(&show["capabilities"], "vision")));
        }
        let ctx = show["model_info"].as_object().and_then(|o| {
            o.iter()
                .find(|(k, _)| k.ends_with(".context_length"))
                .and_then(|(_, v)| v.as_u64())
        });
        m.context_window = m.context_window.or(ctx);
    }
}

/// A local model server found on this machine.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LocalServer {
    /// `ollama` or `lmstudio`.
    pub kind: String,
    /// OpenAI-compatible base URL to configure (`http://127.0.0.1:11434/v1`).
    pub base_url: String,
    pub models: Vec<String>,
}

/// Well-known local servers and their OpenAI-compatible base URLs.
pub const KNOWN_LOCAL: [(&str, &str); 2] = [
    ("ollama", "http://127.0.0.1:11434/v1"),
    ("lmstudio", "http://127.0.0.1:1234/v1"),
];

/// Finds the well-known local servers that answer `GET <base>/models` (short timeout; nothing found = empty).
pub async fn discover(http: &Http) -> Vec<LocalServer> {
    discover_at(http, &KNOWN_LOCAL).await
}

pub async fn discover_at(http: &Http, candidates: &[(&str, &str)]) -> Vec<LocalServer> {
    let mut out = Vec::new();
    for (kind, base) in candidates {
        let r = http
            .client()
            .get(format!("{}/models", base.trim_end_matches('/')))
            .timeout(Duration::from_millis(1500))
            .send()
            .await;
        let Ok(r) = r else { continue };
        if !r.status().is_success() {
            continue;
        }
        let Ok(v) = r.json::<Value>().await else {
            continue;
        };
        let models = v["data"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m["id"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        out.push(LocalServer {
            kind: (*kind).to_owned(),
            base_url: (*base).to_owned(),
            models,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_root() {
        assert_eq!(root("http://localhost:11434/v1"), "http://localhost:11434");
        assert_eq!(root("http://localhost:1234/v1/"), "http://localhost:1234");
        assert_eq!(root("http://host:8080"), "http://host:8080");
    }
}
