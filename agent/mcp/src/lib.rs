//! McpHub (M15-3): every agent gets its own connections to the MCP servers it is bound to; credentials come from
//! the vault at connect time (per-agent credential binding); MCP tools are offered to the model as
//! `mcp__<server>__<tool>` and their output is marked untrusted (decision 34).
//!
//! Reliability: a stdio server that died is restarted on the next call; read-only tools (`readOnlyHint`) are
//! retried once after a reconnect, tools with side effects are not (the error goes back to the model).
//! Not built yet: the legacy HTTP+SSE transport (servers older than revision 2025-03-26) and OAuth.

pub mod client;
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

/// Resolves `${secret:<name>}` and `${cred:<KEY>}` placeholders.
pub fn resolve(
    value: &str,
    secret: &dyn Fn(&str) -> Option<String>,
    creds: &BTreeMap<String, String>,
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
        let name = if let Some(n) = token.strip_prefix("secret:") {
            n.to_owned()
        } else if let Some(k) = token.strip_prefix("cred:") {
            creds.get(k).cloned().ok_or_else(|| {
                McpError::Config(format!(
                    "the agent's binding maps no secret to credential {k:?}"
                ))
            })?
        } else {
            out.push_str(&rest[i..i + 2 + end + 1]);
            rest = &after[end + 1..];
            continue;
        };
        let v = secret(&name)
            .ok_or_else(|| McpError::Config(format!("secret {name:?} is not in the vault")))?;
        out.push_str(&v);
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
            .map(|(k, v)| Ok((k.clone(), resolve(v, secret, &binding.credentials)?)))
            .collect()
    };
    match (&cfg.command, &cfg.url) {
        (Some(command), None) => Ok(ConnectSpec::Stdio {
            command: command.clone(),
            args: cfg
                .args
                .iter()
                .map(|a| resolve(a, secret, &binding.credentials))
                .collect::<Result<_, _>>()?,
            env: map(&cfg.env)?,
            cwd: work_dir,
        }),
        (None, Some(url)) => Ok(ConnectSpec::Http {
            url: url.clone(),
            headers: map(&cfg.headers)?,
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

/// An MCP tool offered to the model.
pub struct McpTool {
    session: Arc<Session>,
    info: ToolInfo,
    name: String,
    description: String,
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
type OpenSession = (Arc<Session>, Vec<ToolInfo>);

pub struct McpHub {
    secret: SecretFn,
    work_dir: PathBuf,
    sessions: tokio::sync::Mutex<HashMap<SessionKey, OpenSession>>,
}

impl McpHub {
    /// `work_dir`: parent of each agent's working folder for stdio servers (`<work_dir>/<agent>/<server>`).
    pub fn new(secret: SecretFn, work_dir: impl Into<PathBuf>) -> Self {
        Self {
            secret,
            work_dir: work_dir.into(),
            sessions: tokio::sync::Mutex::new(HashMap::new()),
        }
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
            let entry = { self.sessions.lock().await.get(&key).cloned() };
            let (session, tools) = match entry {
                Some(e) => e,
                None => match self.open(agent, cfg, b).await {
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
            for t in tools {
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
                }));
            }
        }
        out.tools.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    async fn open(
        &self,
        agent: &str,
        cfg: &McpServerConfig,
        b: &Binding,
    ) -> Result<(Arc<Session>, Vec<ToolInfo>), McpError> {
        let spec = connect_spec(
            cfg,
            b,
            self.secret.as_ref(),
            self.work_dir.join(agent).join(&cfg.id),
        )?;
        let session = Arc::new(Session {
            server: cfg.id.clone(),
            spec,
            timeout: Duration::from_secs(cfg.timeout_secs.max(1)),
            client: tokio::sync::Mutex::new(None),
        });
        let c = session.client().await?;
        let tools = c.list_tools(CONNECT_TIMEOUT).await?;
        Ok((session, tools))
    }

    /// "Test connection": connect, list tools, disconnect.
    pub async fn test(
        &self,
        cfg: &McpServerConfig,
        binding: &Binding,
    ) -> Result<(Value, String, Vec<ToolInfo>), McpError> {
        let spec = connect_spec(
            cfg,
            binding,
            self.secret.as_ref(),
            self.work_dir.join("_test").join(&cfg.id),
        )?;
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
