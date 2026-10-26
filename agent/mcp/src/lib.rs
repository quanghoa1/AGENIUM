//! McpHub (M15-3): every agent gets its own connections to the MCP servers it is bound to; credentials come from
//! the vault at connect time (per-agent credential binding); MCP tools are offered to the model as
//! `mcp__<server>__<tool>` and their output is marked untrusted (decision 34).
//!
//! Reliability: a stdio server that died is restarted on the next call; read-only tools (`readOnlyHint`) are
//! retried once after a reconnect, tools with side effects are not (the error goes back to the model).
//! Remote servers that ask for a signed-in user get one OAuth token per server ([`oauth`], ADR-97).
//! Servers of revision 2024-11-05 are reached through the legacy HTTP+SSE transport when they refuse Streamable HTTP;
//! a server's resources and prompts are offered as read-only tools.

pub mod client;
pub mod oauth;
pub mod server;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agenium_config::McpServerConfig;
use agenium_tools::{Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use client::{CallResult, Client, ConnectSpec, McpError, ToolInfo};
pub use oauth::{MemoryTokens, TokenStore, Tokens};

/// Limits of tool names on the vendor side (OpenAI: 64, `[A-Za-z0-9_-]`).
pub const MAX_TOOL_NAME: usize = 64;
pub const MAX_DESCRIPTION: usize = 1024;
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// One agent's use of a server (stored in the agent's `mcp` field).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub server: String,
    /// `${cred:KEY}` placeholders of the server config -> vault secret names for this agent.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub credentials: BTreeMap<String, String>,
    /// The agent's own keys, stored as they are (the user sees them in the app; logs mask them). A name is an
    /// `env` / `headers` entry of the server (the key replaces the placeholder there, or the whole value when it
    /// has none) or a `${cred:KEY}` placeholder. Entries without an own key use the server's (system) value.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: BTreeMap<String, String>,
    /// Only these tools (by MCP name); `None` = every tool of the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
}

pub fn parse_bindings(v: &Value) -> Result<Vec<Binding>, McpError> {
    if v.is_null() {
        return Ok(vec![]);
    }
    serde_json::from_value(v.clone())
        .map_err(|e| McpError::Config(format!("agent MCP bindings: {e}")))
}

/// Looks up a vault secret by name (exposed value).
pub type SecretFn = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

impl Binding {
    /// The agent's own value for a key name: its literal key, else the vault secret its credential names.
    pub fn own(&self, name: &str, secret: &dyn Fn(&str) -> Option<String>) -> Option<String> {
        self.keys
            .get(name)
            .cloned()
            .or_else(|| self.credentials.get(name).and_then(|n| secret(n)))
    }
}

/// Resolves `${secret:<name>}` (vault) and `${cred:<KEY>}` (the agent's own key, see [`Binding::own`]).
pub fn resolve(
    value: &str,
    secret: &dyn Fn(&str) -> Option<String>,
    own: &dyn Fn(&str) -> Option<String>,
) -> Result<String, McpError> {
    let mut out = String::new();
    let mut rest = value;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[i..]);
            return Ok(out);
        };
        let token = &after[..end];
        let v = if let Some(n) = token.strip_prefix("secret:") {
            secret(n)
                .ok_or_else(|| McpError::Config(format!("secret {n:?} is not in the vault")))?
        } else if let Some(k) = token.strip_prefix("cred:") {
            own(k).ok_or_else(|| McpError::Config(format!("the agent has no key {k:?}")))?
        } else {
            out.push_str(&rest[i..i + 2 + end + 1]);
            rest = &after[end + 1..];
            continue;
        };
        out.push_str(&v);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The key names an agent can set for a server: its `env` and `headers` entries and its `${cred:KEY}` placeholders.
pub fn key_names(cfg: &McpServerConfig) -> Vec<String> {
    let mut out: Vec<String> = cfg.env.keys().chain(cfg.headers.keys()).cloned().collect();
    for v in cfg
        .env
        .values()
        .chain(cfg.headers.values())
        .chain(cfg.args.iter())
    {
        let mut rest = v.as_str();
        while let Some(i) = rest.find("${cred:") {
            let after = &rest[i + 7..];
            let Some(end) = after.find('}') else { break };
            out.push(after[..end].to_owned());
            rest = &after[end + 1..];
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The value of one `env` / `headers` entry for an agent: with an own key for that entry, the key replaces the
/// entry's placeholders (keeping a scheme such as `Bearer `), or the whole value when it has none; without one,
/// the server's (system) value.
fn entry_value(
    name: &str,
    template: &str,
    secret: &dyn Fn(&str) -> Option<String>,
    binding: &Binding,
) -> Result<String, McpError> {
    let own = |k: &str| binding.own(k, secret);
    let Some(value) = own(name) else {
        return resolve(template, secret, &own);
    };
    if !template.contains("${") {
        return Ok(value);
    }
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[i..]);
            return Ok(out);
        };
        let token = &after[..end];
        if token.starts_with("secret:") || token.starts_with("cred:") {
            out.push_str(&value);
        } else {
            out.push_str(&rest[i..i + 2 + end + 1]);
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Builds the connection spec of a server for one agent.
pub fn connect_spec(
    cfg: &McpServerConfig,
    binding: &Binding,
    secret: &dyn Fn(&str) -> Option<String>,
    work_dir: PathBuf,
) -> Result<ConnectSpec, McpError> {
    let map = |m: &BTreeMap<String, String>| -> Result<BTreeMap<String, String>, McpError> {
        m.iter()
            .map(|(k, v)| Ok((k.clone(), entry_value(k, v, secret, binding)?)))
            .collect()
    };
    match (&cfg.command, &cfg.url) {
        (Some(command), None) => Ok(ConnectSpec::Stdio {
            command: command.clone(),
            args: cfg
                .args
                .iter()
                .map(|a| resolve(a, secret, &|k| binding.own(k, secret)))
                .collect::<Result<_, _>>()?,
            env: map(&cfg.env)?,
            cwd: work_dir,
        }),
        (None, Some(url)) => Ok(ConnectSpec::Http {
            url: url.clone(),
            headers: map(&cfg.headers)?,
            bearer: None,
        }),
        _ => Err(McpError::Config(format!(
            "server {:?} needs exactly one of command or url",
            cfg.id
        ))),
    }
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// `mcp__<server>__<tool>` with characters vendors accept; long names keep a hash suffix so they stay unique.
pub fn exposed_name(server: &str, tool: &str) -> String {
    let clean: String = tool
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let full = format!("mcp__{server}__{clean}");
    if full.len() <= MAX_TOOL_NAME && clean == tool {
        return full;
    }
    let suffix = format!("_{:08x}", fnv(tool) as u32);
    let keep = MAX_TOOL_NAME.saturating_sub(suffix.len()).min(full.len());
    format!("{}{suffix}", &full[..keep])
}

/// One agent's connection to one server; reconnects lazily.
struct Session {
    server: String,
    spec: ConnectSpec,
    timeout: Duration,
    client: tokio::sync::Mutex<Option<Arc<Client>>>,
}

impl Session {
    async fn client(&self) -> Result<Arc<Client>, McpError> {
        let mut g = self.client.lock().await;
        if let Some(c) = g.as_ref() {
            if c.alive() {
                return Ok(c.clone());
            }
            tracing::info!(server = %self.server, "MCP server connection lost; reconnecting");
        }
        let c = Arc::new(Client::connect(&self.spec, CONNECT_TIMEOUT).await?);
        *g = Some(c.clone());
        Ok(c)
    }

    async fn reset(&self) {
        *self.client.lock().await = None;
    }
}

/// The server's resources and prompts, offered as read-only tools when it declares them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extra {
    ListResources,
    ReadResource,
    ListPrompts,
    GetPrompt,
}

impl Extra {
    /// The extras a server's declared capabilities allow.
    pub fn of(capabilities: &Value) -> Vec<Extra> {
        let mut v = Vec::new();
        if capabilities.get("resources").is_some() {
            v.extend([Extra::ListResources, Extra::ReadResource]);
        }
        if capabilities.get("prompts").is_some() {
            v.extend([Extra::ListPrompts, Extra::GetPrompt]);
        }
        v
    }

    fn info(self) -> ToolInfo {
        let (name, description, schema) = match self {
            Extra::ListResources => (
                "list_resources",
                "List the server's resources (uri, name, description).",
                serde_json::json!({"type": "object", "properties": {}}),
            ),
            Extra::ReadResource => (
                "read_resource",
                "Read one of the server's resources by its uri.",
                serde_json::json!({"type": "object", "properties": {"uri": {"type": "string"}}, "required": ["uri"]}),
            ),
            Extra::ListPrompts => (
                "list_prompts",
                "List the server's prompt templates and their arguments.",
                serde_json::json!({"type": "object", "properties": {}}),
            ),
            Extra::GetPrompt => (
                "get_prompt",
                "Get one of the server's prompt templates filled with arguments.",
                serde_json::json!({"type": "object", "properties": {"name": {"type": "string"}, "arguments": {"type": "object"}}, "required": ["name"]}),
            ),
        };
        ToolInfo {
            name: name.into(),
            description: description.into(),
            input_schema: schema,
            read_only: true,
        }
    }
}

/// An MCP tool offered to the model.
pub struct McpTool {
    session: Arc<Session>,
    info: ToolInfo,
    name: String,
    description: String,
    /// A resources/prompts tool rather than one of the server's own tools.
    extra: Option<Extra>,
}

impl McpTool {
    pub fn mcp_name(&self) -> &str {
        &self.info.name
    }
}

fn tool_error(e: McpError) -> ToolError {
    match e {
        McpError::Timeout(d) => ToolError::new(
            "timeout",
            format!("the MCP tool did not answer within {d:?}"),
        ),
        McpError::Rpc { code, message } => {
            ToolError::new("mcp_error", format!("{message} (code {code})"))
        }
        McpError::AuthRequired(why) => ToolError::new(
            "mcp_needs_sign_in",
            format!("{why}; only the user can sign in, tell them"),
        ),
        other => ToolError::new("mcp_unavailable", other.to_string()),
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn parameters(&self) -> Value {
        self.info.input_schema.clone()
    }
    fn read_only(&self) -> bool {
        self.info.read_only
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput, ToolError> {
        if let Some(extra) = self.extra {
            let c = self.session.client().await.map_err(tool_error)?;
            let t = self.session.timeout;
            let text = match extra {
                Extra::ListResources => c.list_resources(t).await,
                Extra::ReadResource => match args["uri"].as_str() {
                    Some(uri) => c.read_resource(uri, t).await,
                    None => return Err(ToolError::new("invalid_arguments", "uri is required")),
                },
                Extra::ListPrompts => c.list_prompts(t).await,
                Extra::GetPrompt => match args["name"].as_str() {
                    Some(name) => c.get_prompt(name, args["arguments"].clone(), t).await,
                    None => return Err(ToolError::new("invalid_arguments", "name is required")),
                },
            }
            .map_err(tool_error)?;
            return Ok(ctx.finish_output(text, true));
        }
        let attempt = |c: Arc<Client>, a: Value| async move {
            c.call_tool(&self.info.name, a, self.session.timeout).await
        };
        let c = self.session.client().await.map_err(tool_error)?;
        let r = match attempt(c, args.clone()).await {
            Err(McpError::Transport(why)) => {
                self.session.reset().await;
                if !self.info.read_only {
                    return Err(ToolError::new(
                        "mcp_unavailable",
                        format!(
                            "the connection to MCP server {} failed during the call ({why}); the call may or may not \
                             have run, check its effect before calling it again",
                            self.session.server
                        ),
                    ));
                }
                let c = self.session.client().await.map_err(tool_error)?;
                attempt(c, args).await
            }
            r => r,
        }
        .map_err(tool_error)?;
        if r.is_error {
            return Err(ToolError::new("mcp_tool_error", r.text));
        }
        Ok(ctx.finish_output(r.text, true))
    }
}

/// What a run gets from the hub.
#[derive(Default)]
pub struct AgentTools {
    pub tools: Vec<Arc<McpTool>>,
    /// `server: problem` lines for the manifest (unknown server, missing secret, connection failure).
    pub problems: Vec<String>,
    /// Connected server ids.
    pub servers: Vec<String>,
}

/// (agent, server)
type SessionKey = (String, String);
/// The session, its tools and the fingerprint of the resolved connection (a changed key reconnects).
type OpenSession = (Arc<Session>, Vec<ToolInfo>, u64, Vec<Extra>);

/// In-memory fingerprint of a resolved connection (values included, never stored or printed).
fn fingerprint(spec: &ConnectSpec) -> u64 {
    let mut s = String::new();
    match spec {
        ConnectSpec::Stdio {
            command, args, env, ..
        } => {
            s.push_str(command);
            for a in args {
                s.push('\0');
                s.push_str(a);
            }
            for (k, v) in env {
                s.push_str(&format!("\0{k}={v}"));
            }
        }
        ConnectSpec::Http { url, headers, .. } => {
            s.push_str(url);
            for (k, v) in headers {
                s.push_str(&format!("\0{k}={v}"));
            }
        }
    }
    fnv(&s)
}

pub struct McpHub {
    secret: SecretFn,
    work_dir: PathBuf,
    sessions: tokio::sync::Mutex<HashMap<SessionKey, OpenSession>>,
    tokens: Option<Arc<dyn TokenStore>>,
    /// One OAuth token holder per server, shared by every agent's connection to it.
    bearers: std::sync::Mutex<HashMap<String, Arc<oauth::Bearer>>>,
}

impl McpHub {
    /// `work_dir`: parent of each agent's working folder for stdio servers (`<work_dir>/<agent>/<server>`).
    pub fn new(secret: SecretFn, work_dir: impl Into<PathBuf>) -> Self {
        Self {
            secret,
            work_dir: work_dir.into(),
            sessions: tokio::sync::Mutex::new(HashMap::new()),
            tokens: None,
            bearers: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Where OAuth tokens are kept; without a store, servers that ask for sign-in cannot be used.
    pub fn with_tokens(mut self, store: Arc<dyn TokenStore>) -> Self {
        self.tokens = Some(store);
        self
    }

    /// The server's OAuth token goes on its HTTP connection unless the config sends its own `Authorization`.
    fn spec_for(
        &self,
        cfg: &McpServerConfig,
        binding: &Binding,
        work_dir: PathBuf,
    ) -> Result<ConnectSpec, McpError> {
        let mut spec = connect_spec(cfg, binding, self.secret.as_ref(), work_dir)?;
        if let (
            ConnectSpec::Http {
                url,
                headers,
                bearer,
            },
            Some(store),
        ) = (&mut spec, &self.tokens)
        {
            if !headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case("authorization"))
            {
                let mut all = self.bearers.lock().unwrap_or_else(|e| e.into_inner());
                let b = all
                    .entry(cfg.id.clone())
                    .or_insert_with(|| Arc::new(oauth::Bearer::new(&cfg.id, url, store.clone())));
                if !b.matches(url) {
                    *b = Arc::new(oauth::Bearer::new(&cfg.id, url, store.clone()));
                }
                *bearer = Some(b.clone());
            }
        }
        Ok(spec)
    }

    /// Signs the user in to an HTTP server (`open` shows them the authorization URL) and keeps the tokens.
    pub async fn login(
        &self,
        cfg: &McpServerConfig,
        scopes: &[String],
        open: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<Tokens, McpError> {
        let store = self
            .tokens
            .clone()
            .ok_or_else(|| McpError::Config("no token store".into()))?;
        let url = cfg
            .url
            .as_deref()
            .ok_or_else(|| McpError::Config(format!("server {:?} has no url", cfg.id)))?;
        let o = cfg.oauth.clone().unwrap_or_default();
        let client = match &o.client_id {
            Some(id) => Some(oauth::OAuthClient {
                client_id: id.clone(),
                client_secret: o
                    .client_secret
                    .as_deref()
                    .map(|s| resolve(s, self.secret.as_ref(), &|_| None))
                    .transpose()?,
            }),
            None => None,
        };
        let opts = oauth::LoginOptions {
            client,
            scopes: if scopes.is_empty() {
                o.scopes.clone()
            } else {
                scopes.to_vec()
            },
            callback_port: o.callback_port.unwrap_or(0),
            timeout: None,
        };
        let t = oauth::login(&cfg.id, url, &opts, store.as_ref(), open).await?;
        self.forget_server(&cfg.id).await;
        Ok(t)
    }

    /// Forgets the server's tokens and drops its connections.
    pub async fn logout(&self, server: &str) -> Result<(), McpError> {
        if let Some(store) = &self.tokens {
            store.remove(server).map_err(McpError::Auth)?;
        }
        self.forget_server(server).await;
        Ok(())
    }

    /// Whether tokens for this server's URL are stored.
    pub fn signed_in(&self, cfg: &McpServerConfig) -> bool {
        match (&self.tokens, &cfg.url) {
            (Some(store), Some(url)) => store.load(&cfg.id).is_some_and(|t| &t.server_url == url),
            _ => false,
        }
    }

    async fn forget_server(&self, server: &str) {
        self.bearers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(server);
        self.sessions.lock().await.retain(|(_, s), _| s != server);
    }

    /// Connects (once per agent and server) and lists the tools an agent may use.
    pub async fn tools_for(
        &self,
        agent: &str,
        bindings: &[Binding],
        servers: &[McpServerConfig],
    ) -> AgentTools {
        let mut out = AgentTools::default();
        for b in bindings {
            let Some(cfg) = servers.iter().find(|s| s.id == b.server) else {
                out.problems
                    .push(format!("{}: no such MCP server is configured", b.server));
                continue;
            };
            let key = (agent.to_owned(), cfg.id.clone());
            let spec = match self.spec_for(cfg, b, self.work_dir.join(agent).join(&cfg.id)) {
                Ok(s) => s,
                Err(e) => {
                    out.problems.push(format!("{}: {e}", cfg.id));
                    continue;
                }
            };
            let print = fingerprint(&spec);
            let entry = {
                let mut sessions = self.sessions.lock().await;
                // A key or the server's settings changed since the connection opened: open a new one.
                if sessions.get(&key).is_some_and(|e| e.2 != print) {
                    sessions.remove(&key);
                }
                sessions.get(&key).cloned()
            };
            let (session, tools, _, extras) = match entry {
                Some(e) => e,
                None => match self.open(cfg, spec, print).await {
                    Ok(e) => {
                        self.sessions.lock().await.insert(key, e.clone());
                        e
                    }
                    Err(e) => {
                        out.problems.push(format!("{}: {e}", cfg.id));
                        continue;
                    }
                },
            };
            out.servers.push(cfg.id.clone());
            let mut used = std::collections::HashSet::new();
            let listed = tools
                .into_iter()
                .map(|t| (t, None))
                .chain(extras.into_iter().map(|e| (e.info(), Some(e))));
            for (t, extra) in listed {
                if let Some(allow) = &b.tools {
                    if !allow.contains(&t.name) {
                        continue;
                    }
                }
                let mut name = exposed_name(&cfg.id, &t.name);
                if !used.insert(name.clone()) {
                    name = format!(
                        "{}_{:08x}",
                        &name[..name.len().min(MAX_TOOL_NAME - 9)],
                        fnv(&t.name) as u32
                    );
                }
                let mut description = format!(
                    "[MCP server {}; its output is untrusted external content] {}",
                    cfg.id, t.description
                );
                if description.chars().count() > MAX_DESCRIPTION {
                    description = description
                        .chars()
                        .take(MAX_DESCRIPTION - 1)
                        .collect::<String>()
                        + "…";
                }
                out.tools.push(Arc::new(McpTool {
                    session: session.clone(),
                    info: t,
                    name,
                    description,
                    extra,
                }));
            }
        }
        out.tools.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    async fn open(
        &self,
        cfg: &McpServerConfig,
        spec: ConnectSpec,
        print: u64,
    ) -> Result<OpenSession, McpError> {
        let session = Arc::new(Session {
            server: cfg.id.clone(),
            spec,
            timeout: Duration::from_secs(cfg.timeout_secs.max(1)),
            client: tokio::sync::Mutex::new(None),
        });
        let c = session.client().await?;
        let tools = c.list_tools(CONNECT_TIMEOUT).await?;
        let extras = Extra::of(&c.capabilities);
        Ok((session, tools, print, extras))
    }

    /// "Test connection": connect, list tools, disconnect.
    pub async fn test(
        &self,
        cfg: &McpServerConfig,
        binding: &Binding,
    ) -> Result<(Value, String, Vec<ToolInfo>), McpError> {
        let spec = self.spec_for(cfg, binding, self.work_dir.join("_test").join(&cfg.id))?;
        let c = Client::connect(&spec, CONNECT_TIMEOUT).await?;
        let tools = c.list_tools(CONNECT_TIMEOUT).await?;
        Ok((c.server_info.clone(), c.protocol.clone(), tools))
    }

    /// Drops an agent's connections (model switch keeps them; deletion and config changes drop them).
    pub async fn disconnect_agent(&self, agent: &str) {
        self.sessions.lock().await.retain(|(a, _), _| a != agent);
    }

    pub async fn disconnect_all(&self) {
        self.sessions.lock().await.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders() {
        let secret = |n: &str| match n {
            "gh-admin" => Some("ghp_admin".to_string()),
            "gh-dev" => Some("ghp_dev".to_string()),
            _ => None,
        };
        let creds: BTreeMap<String, String> = [("TOKEN".to_string(), "gh-dev".to_string())].into();
        let creds = |k: &str| creds.get(k).and_then(|n| secret(n));
        assert_eq!(
            resolve("Bearer ${cred:TOKEN}", &secret, &creds).unwrap(),
            "Bearer ghp_dev"
        );
        assert_eq!(
            resolve("${secret:gh-admin}", &secret, &creds).unwrap(),
            "ghp_admin"
        );
        assert_eq!(
            resolve("plain ${HOME} $x", &secret, &creds).unwrap(),
            "plain ${HOME} $x"
        );
        assert!(resolve("${secret:nope}", &secret, &creds).is_err());
        assert!(resolve("${cred:OTHER}", &secret, &creds).is_err());
        assert_eq!(
            resolve("${unclosed", &secret, &creds).unwrap(),
            "${unclosed"
        );
    }

    #[test]
    fn own_keys_replace_the_system_key_per_entry() {
        let secret = |n: &str| (n == "mcp-docs-token").then(|| "system".to_string());
        let mut b = Binding {
            server: "docs".into(),
            credentials: BTreeMap::new(),
            keys: BTreeMap::new(),
            tools: None,
        };
        let auth = "Bearer ${secret:mcp-docs-token}";
        assert_eq!(
            entry_value("Authorization", auth, &secret, &b).unwrap(),
            "Bearer system"
        );
        assert_eq!(
            entry_value("API_KEY", "shared", &secret, &b).unwrap(),
            "shared"
        );
        b.keys.insert("Authorization".into(), "mine".into());
        b.keys.insert("API_KEY".into(), "own".into());
        b.keys.insert("PERSONAL".into(), "p".into());
        assert_eq!(
            entry_value("Authorization", auth, &secret, &b).unwrap(),
            "Bearer mine"
        );
        assert_eq!(
            entry_value("API_KEY", "shared", &secret, &b).unwrap(),
            "own"
        );
        assert_eq!(
            entry_value("X", "k=${cred:PERSONAL}", &secret, &b).unwrap(),
            "k=p"
        );
        let cfg: McpServerConfig = serde_json::from_value(serde_json::json!({
            "id": "docs", "url": "http://x", "headers": {"Authorization": "Bearer ${secret:t}"},
            "env": {"API_KEY": "${cred:PERSONAL}"}
        }))
        .unwrap();
        assert_eq!(key_names(&cfg), ["API_KEY", "Authorization", "PERSONAL"]);
    }

    #[test]
    fn names() {
        assert_eq!(
            exposed_name("github", "create_issue"),
            "mcp__github__create_issue"
        );
        let n = exposed_name("fs", "read.file");
        assert!(
            n.starts_with("mcp__fs__read_file_") && n.len() <= MAX_TOOL_NAME,
            "{n}"
        );
        let long = exposed_name("server", &"x".repeat(100));
        assert_eq!(long.len(), MAX_TOOL_NAME);
        assert_ne!(
            exposed_name("s", &"a".repeat(80)),
            exposed_name("s", &format!("{}b", "a".repeat(79)))
        );
    }

    #[test]
    fn bindings_parse() {
        let v = serde_json::json!([{"server": "github", "credentials": {"TOKEN": "gh-dev"}, "tools": ["create_issue"]}]);
        let b = parse_bindings(&v).unwrap();
        assert_eq!(b[0].server, "github");
        assert!(parse_bindings(&serde_json::json!([{"server": "x", "bogus": 1}])).is_err());
        assert!(parse_bindings(&serde_json::json!([])).unwrap().is_empty());
    }
}
