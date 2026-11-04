//! MCP settings (M15-3): servers in `config.toml`, agent bindings in the agent row. Keys (owner's decision,
//! 2026-10-25): stored as they are, the server's value is the system key and an agent may have its own key per
//! entry (`Binding::keys`); the app shows them, logs mask them. `${secret:<name>}` vault references still work.

use std::collections::BTreeMap;
use std::sync::Arc;

use agenium_config::McpServerConfig;
use agenium_core::*;
use agenium_mcp::{Binding, McpHub, SecretFn, TokenStore, Tokens};
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

/// OAuth tokens of MCP servers, one vault secret per server (`mcp-oauth/<server>`, JSON). Values read or written
/// are registered with the redactor (ADR-97).
pub struct VaultTokens(pub Arc<App>);

fn token_name(server: &str) -> String {
    format!("mcp-oauth/{server}")
}

impl VaultTokens {
    fn mask(&self, t: &Tokens) {
        let r = self.0.redactor();
        r.register("mcp-oauth-access", &t.access_token);
        if let Some(x) = &t.refresh_token {
            r.register("mcp-oauth-refresh", x);
        }
        if let Some(x) = &t.client_secret {
            r.register("mcp-oauth-client", x);
        }
    }
}

impl TokenStore for VaultTokens {
    fn load(&self, server: &str) -> Option<Tokens> {
        let s = self.0.secrets().get(&token_name(server)).ok()??;
        let t: Tokens = serde_json::from_str(s.expose_str().ok()?).ok()?;
        self.mask(&t);
        Some(t)
    }

    fn save(&self, server: &str, tokens: &Tokens) -> std::result::Result<(), String> {
        self.mask(tokens);
        let json = serde_json::to_string(tokens).map_err(|e| e.to_string())?;
        self.0
            .secrets()
            .set(&token_name(server), json.as_bytes())
            .map_err(|e| e.to_string())
    }

    fn remove(&self, server: &str) -> std::result::Result<(), String> {
        self.0
            .secrets()
            .delete(&token_name(server))
            .map_err(|e| e.to_string())
    }
}

/// The MCP hub of this root: vault secrets for `${secret:…}`, vault-kept OAuth tokens.
pub fn hub(app: Arc<App>) -> McpHub {
    McpHub::new(secret_fn(app.clone()), app.layout().mcp_work())
        .with_tokens(Arc::new(VaultTokens(app)))
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

/// Plain values that look like secrets (masked in logs).
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
}

/// One key an agent can set for a server it uses.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AgentMcpKey {
    pub server: String,
    /// The `env` / `headers` entry (or `${cred:KEY}` placeholder).
    pub name: String,
    /// The server's value (the system key), as configured.
    pub system: Option<String>,
    /// The agent's own key; `None` = the agent uses the system key.
    pub own: Option<String>,
}

/// A key value and, for `Scheme value` (`Bearer abc`), the value alone: what the log redactor must know.
fn mask_forms(v: &str) -> Vec<&str> {
    let mut out = vec![v];
    if let Some((_, rest)) = v.split_once(' ') {
        out.push(rest);
    }
    out
}

/// The servers of an `mcpServers` object (`{name: {command, args, env} | {url, headers}}`), each with the id its name
/// gives, values kept as written.
pub(crate) fn servers_from_json(map: &serde_json::Value) -> Result<Vec<(String, McpServerConfig)>> {
    let map = map
        .as_object()
        .ok_or_else(|| OrchError::Invalid("expected an object \"mcpServers\"".into()))?;
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
        let cfg = McpServerConfig {
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
            oauth: None,
            approval: None,
        };
        configs.push((name.clone(), cfg));
    }
    Ok(configs)
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
    /// Adds or replaces an MCP server. Keys are kept as written (the system key of the server).
    pub fn put_mcp_server(&self, cfg: McpServerConfig) -> Result<()> {
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
        // Its OAuth tokens go with it.
        self.secrets().delete(&token_name(id))?;
        Ok(())
    }

    /// Imports `{"mcpServers": {name: {command, args, env} | {url, headers}}}` (the format most MCP docs show).
    /// Values are kept as written.
    pub fn import_mcp_json(&self, text: &str) -> Result<JsonImport> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let v: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| OrchError::Invalid(format!("not valid JSON: {e}")))?;
        let map = v
            .get("mcpServers")
            .or_else(|| v.get("servers"))
            .ok_or_else(|| OrchError::Invalid("expected an object \"mcpServers\"".into()))?;
        let configs: Vec<McpServerConfig> = servers_from_json(map)?
            .into_iter()
            .map(|(_, c)| c)
            .collect();
        let out = JsonImport {
            servers: configs.iter().map(|c| c.id.clone()).collect(),
        };
        self.update_config(|c| {
            for cfg in configs {
                c.mcp.retain(|m| m.id != cfg.id);
                c.mcp.push(cfg);
            }
            c.mcp.sort_by(|a, b| a.id.cmp(&b.id));
        })?;
        Ok(out)
    }

    /// Tells the log redactor the key values an agent's MCP connections use (system keys written as plain values
    /// and the agent's own keys): the app shows them, logs never do.
    pub fn mask_mcp_keys(&self, bindings: &[Binding]) {
        let cfg = self.config();
        for b in bindings {
            if let Some(s) = cfg.mcp_server(&b.server) {
                for (k, v) in s.env.iter().chain(s.headers.iter()) {
                    if !v.contains("${") && looks_secret(k, v) {
                        for f in mask_forms(v) {
                            self.redactor.register(&format!("mcp/{}/{k}", s.id), f);
                        }
                    }
                }
            }
            for (k, v) in &b.keys {
                for f in mask_forms(v) {
                    self.redactor
                        .register(&format!("mcp/{}/{}/own", b.server, k), f);
                }
            }
        }
    }

    /// The keys of every MCP server an agent uses: the system value and the agent's own key.
    pub fn agent_mcp_keys(&self, id: AgentId) -> Result<Vec<AgentMcpKey>> {
        let a = self.agent(id)?;
        let cfg = self.config();
        let mut out = Vec::new();
        for b in
            agenium_mcp::parse_bindings(&a.mcp).map_err(|e| OrchError::Invalid(e.to_string()))?
        {
            let Some(s) = cfg.mcp_server(&b.server) else {
                continue;
            };
            for name in agenium_mcp::key_names(s) {
                out.push(AgentMcpKey {
                    server: b.server.clone(),
                    system: s.env.get(&name).or_else(|| s.headers.get(&name)).cloned(),
                    own: b
                        .keys
                        .get(&name)
                        .cloned()
                        .or_else(|| b.credentials.get(&name).map(|n| format!("${{secret:{n}}}"))),
                    name,
                });
            }
        }
        Ok(out)
    }

    /// Gives an agent its own key for one entry of a server it uses (`Some`), or removes it (`None`: back to
    /// the system key). `actor` is who did it, for the event log (`user` or `agent:<id>`).
    pub fn set_agent_mcp_key(
        &self,
        id: AgentId,
        server: &str,
        name: &str,
        value: Option<&str>,
        actor: &str,
    ) -> Result<()> {
        let a = self.agent(id)?;
        let cfg = self.config();
        let s = cfg
            .mcp_server(server)
            .ok_or_else(|| OrchError::NotFound(format!("MCP server {server}")))?;
        let names = agenium_mcp::key_names(s);
        if !names.iter().any(|n| n == name) {
            return Err(OrchError::Invalid(format!(
                "MCP server {server} has no key {name:?}; its keys: {}",
                names.join(", ")
            )));
        }
        let mut b =
            agenium_mcp::parse_bindings(&a.mcp).map_err(|e| OrchError::Invalid(e.to_string()))?;
        let Some(bind) = b.iter_mut().find(|x| x.server == server) else {
            return Err(OrchError::Conflict(format!(
                "{} does not use MCP server {server}; the user enables it on the agent's page first",
                a.name
            )));
        };
        match value.map(str::trim) {
            Some("") => return Err(OrchError::Invalid("the key is empty".into())),
            Some(v) => {
                bind.keys.insert(name.to_owned(), v.to_owned());
                bind.credentials.remove(name);
            }
            None => {
                bind.keys.remove(name);
                bind.credentials.remove(name);
            }
        }
        self.mask_mcp_keys(&b);
        self.write_agent_mcp(id, &b, actor)
    }

    /// Replaces the agent's MCP bindings (every server must be configured).
    pub fn set_agent_mcp(&self, id: AgentId, bindings: &[Binding]) -> Result<()> {
        self.write_agent_mcp(id, bindings, "user")
    }

    fn write_agent_mcp(&self, id: AgentId, bindings: &[Binding], actor: &str) -> Result<()> {
        let cfg = self.config();
        for b in bindings {
            if cfg.mcp_server(&b.server).is_none() {
                return Err(OrchError::NotFound(format!("MCP server {}", b.server)));
            }
        }
        let v = serde_json::to_value(bindings).map_err(|e| OrchError::Invalid(e.to_string()))?;
        self.store()
            .commit(None, actor, |w| -> std::result::Result<_, StoreError> {
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
