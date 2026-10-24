//! MCP settings (M15-3): servers in `config.toml`, agent bindings in the agent row, credentials in the vault.

use std::collections::BTreeMap;
use std::sync::Arc;

use agenium_config::McpServerConfig;
use agenium_core::*;
use agenium_mcp::{Binding, SecretFn};
use agenium_store::core::agents;
use agenium_store::StoreError;

use crate::app::App;
use crate::{OrchError, Result};

/// Vault lookups for the MCP hub (also registers the values with the redactor).
pub fn secret_fn(app: Arc<App>) -> SecretFn {
    Arc::new(move |name: &str| {
        app.secret(name)
            .ok()
            .flatten()
            .and_then(|s| s.expose_str().ok().map(str::to_owned))
    })
}

fn looks_secret(key: &str, value: &str) -> bool {
    let k = key.to_ascii_uppercase();
    [
        "KEY",
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "AUTH",
        "CREDENTIAL",
        "COOKIE",
    ]
    .iter()
    .any(|w| k.contains(w))
        || !agenium_secrets::scan_text(value).is_empty()
}

/// Plain values that look like secrets (refused in `config.toml`).
pub fn plaintext_secrets(cfg: &McpServerConfig) -> Vec<String> {
    cfg.env
        .iter()
        .chain(cfg.headers.iter())
        .filter(|(k, v)| !v.contains("${") && looks_secret(k, v))
        .map(|(k, _)| k.clone())
        .collect()
}

/// Result of importing pasted MCP JSON.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JsonImport {
    pub servers: Vec<String>,
    /// Vault secrets created from literal values (`mcp-<server>-<key>`).
    pub secrets: Vec<String>,
}

fn server_id(name: &str) -> String {
    let id: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    id.trim_matches('-').chars().take(32).collect()
}

impl App {
    /// Adds or replaces an MCP server. Secret-looking literal values are refused: store them with
    /// `secret set` and reference them as `${secret:<name>}`.
    pub fn put_mcp_server(&self, cfg: McpServerConfig) -> Result<()> {
        let bad = plaintext_secrets(&cfg);
        if !bad.is_empty() {
            return Err(OrchError::Invalid(format!(
                "{} look like secrets; store them in the vault and use ${{secret:<name>}} instead",
                bad.join(", ")
            )));
        }
        self.update_config(|c| {
            c.mcp.retain(|m| m.id != cfg.id);
            c.mcp.push(cfg);
            c.mcp.sort_by(|a, b| a.id.cmp(&b.id));
        })?;
        Ok(())
    }

    /// Removes a server; refused while an agent is bound to it.
    pub fn remove_mcp_server(&self, id: &str) -> Result<()> {
        let users: Vec<String> = self
            .agents()?
            .into_iter()
            .filter(|a| {
                agenium_mcp::parse_bindings(&a.mcp)
                    .map(|b| b.iter().any(|x| x.server == id))
                    .unwrap_or(false)
            })
            .map(|a| a.name)
            .collect();
        if !users.is_empty() {
            return Err(OrchError::Conflict(format!(
                "agents still use MCP server {id}: {}",
                users.join(", ")
            )));
        }
        if self.config().mcp_server(id).is_none() {
            return Err(OrchError::NotFound(format!("MCP server {id}")));
        }
        self.update_config(|c| c.mcp.retain(|m| m.id != id))?;
        Ok(())
    }

    /// Imports `{"mcpServers": {name: {command, args, env} | {url, headers}}}` (the format most MCP docs show).
    /// Literal secret values move to the vault and are referenced as `${secret:mcp-<server>-<key>}`.
    pub fn import_mcp_json(&self, text: &str) -> Result<JsonImport> {
        let v: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| OrchError::Invalid(format!("not valid JSON: {e}")))?;
        let map = v
            .get("mcpServers")
            .or_else(|| v.get("servers"))
            .and_then(|m| m.as_object())
            .ok_or_else(|| OrchError::Invalid("expected an object \"mcpServers\"".into()))?;
        let mut out = JsonImport::default();
        let mut configs = Vec::new();
        for (name, s) in map {
            let id = server_id(name);
            if !agenium_config::valid_mcp_id(&id) {
                return Err(OrchError::Invalid(format!(
                    "cannot derive a server id from {name:?}"
                )));
            }
            let strings = |key: &str| -> BTreeMap<String, String> {
                s.get(key)
                    .and_then(|e| e.as_object())
                    .map(|o| {
                        o.iter()
                            .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let mut cfg = McpServerConfig {
                id: id.clone(),
                command: s.get("command").and_then(|c| c.as_str()).map(str::to_owned),
                args: s
                    .get("args")
                    .and_then(|a| a.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
                env: strings("env"),
                url: s
                    .get("url")
                    .or_else(|| s.get("serverUrl"))
                    .and_then(|u| u.as_str())
                    .map(str::to_owned),
                headers: strings("headers"),
                timeout_secs: 120,
            };
            for (k, v) in cfg.env.iter_mut().chain(cfg.headers.iter_mut()) {
                if v.contains("${") || !looks_secret(k, v) {
                    continue;
                }
                let secret = format!("mcp-{id}-{}", server_id(k));
                // Keep an auth scheme such as "Bearer " in the config; only the credential goes to the vault.
                let (scheme, value) = match v.split_once(' ') {
                    Some((scheme, rest)) if k.eq_ignore_ascii_case("authorization") => {
                        (format!("{scheme} "), rest.to_owned())
                    }
                    _ => (String::new(), v.clone()),
                };
                self.secrets().set(&secret, value.as_bytes())?;
                *v = format!("{scheme}${{secret:{secret}}}");
                out.secrets.push(secret);
            }
            out.servers.push(id);
            configs.push(cfg);
        }
        self.update_config(|c| {
            for cfg in configs {
                c.mcp.retain(|m| m.id != cfg.id);
                c.mcp.push(cfg);
            }
            c.mcp.sort_by(|a, b| a.id.cmp(&b.id));
        })?;
        Ok(out)
    }

    /// Replaces the agent's MCP bindings (every server must be configured).
    pub fn set_agent_mcp(&self, id: AgentId, bindings: &[Binding]) -> Result<()> {
        let cfg = self.config();
        for b in bindings {
            if cfg.mcp_server(&b.server).is_none() {
                return Err(OrchError::NotFound(format!("MCP server {}", b.server)));
            }
        }
        let v = serde_json::to_value(bindings).map_err(|e| OrchError::Invalid(e.to_string()))?;
        self.store()
            .commit(None, "user", |w| -> std::result::Result<_, StoreError> {
                agents::set_agent_mcp(w.tx, id, &v)?;
                w.emit(DomainEvent::AgentUpdated {
                    agent: id,
                    field: "mcp".into(),
                })?;
                Ok(())
            })?;
        Ok(())
    }
}
