//! A server's tools as the model gets them: one agent's connection to a server ([`Session`], reconnecting lazily) and
//! each tool on it ([`McpTool`]), with the server's resources and prompts as read-only tools ([`Extra`]).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use agenium_tools::{Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::Value;

use crate::{Client, ConnectSpec, McpError, ToolInfo, CONNECT_TIMEOUT};

/// One agent's connection to one server; reconnects lazily.
pub(crate) struct Session {
    pub(crate) server: String,
    pub(crate) spec: ConnectSpec,
    pub(crate) timeout: Duration,
    pub(crate) client: tokio::sync::Mutex<Option<Arc<Client>>>,
    /// Set when a lost connection was opened again: the server may list other tools now.
    pub(crate) reconnected: AtomicBool,
}

impl Session {
    pub(crate) fn new(server: &str, spec: ConnectSpec, timeout: Duration) -> Session {
        Session {
            server: server.to_owned(),
            spec,
            timeout,
            client: tokio::sync::Mutex::new(None),
            reconnected: AtomicBool::new(false),
        }
    }

    pub(crate) async fn client(&self) -> Result<Arc<Client>, McpError> {
        let mut g = self.client.lock().await;
        if let Some(c) = g.as_ref() {
            if c.alive() {
                return Ok(c.clone());
            }
            tracing::info!(server = %self.server, "MCP server connection lost; reconnecting");
            self.reconnected.store(true, Ordering::SeqCst);
        }
        let c = Arc::new(Client::connect(&self.spec, CONNECT_TIMEOUT).await?);
        *g = Some(c.clone());
        Ok(c)
    }

    pub(crate) async fn reset(&self) {
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

    pub(crate) fn info(self) -> ToolInfo {
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
    /// The server's tool list (none for a resources/prompts tool): a tool it no longer lists is not called.
    pub(crate) listed: Option<crate::catalog::Listed>,
    pub(crate) session: Arc<Session>,
    pub(crate) info: ToolInfo,
    pub(crate) name: String,
    pub(crate) description: String,
    /// A resources/prompts tool rather than one of the server's own tools.
    pub(crate) extra: Option<Extra>,
}

impl McpTool {
    pub fn mcp_name(&self) -> &str {
        &self.info.name
    }

    /// The id of the server the tool is on.
    pub fn server(&self) -> &str {
        &self.session.server
    }

    /// The tool's output fenced as the server's (ADR-142).
    fn fenced(&self, text: &str) -> String {
        agenium_tools::untrusted::fence(
            &format!(
                "MCP server {}, tool {}",
                self.session.server, self.info.name
            ),
            text,
        )
    }

    /// The server's own description of the tool (without the untrusted-content label the model gets).
    pub fn plain_description(&self) -> &str {
        &self.info.description
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
            let mut out = ctx.finish_output(text, true);
            out.text = self.fenced(&out.text);
            return Ok(out);
        }
        // The server changed its tools during the run and this one is gone (ADR-139): say where the others are.
        if let Some(l) = &self.listed {
            let _ = crate::catalog::relist(&self.session, l, false, false).await;
            let (gone, now) = {
                let g = l.lock().unwrap_or_else(|e| e.into_inner());
                (
                    !g.tools.iter().any(|t| t.name == self.info.name),
                    g.tools.len(),
                )
            };
            if gone {
                return Err(ToolError::new(
                    "tool_gone",
                    format!(
                        "MCP server {} no longer offers {} (its tool list changed during the run; it offers {now} \
                         other tools now): find what you need with tool_search and call it with call_tool",
                        self.session.server, self.info.name
                    ),
                ));
            }
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
        let mut out = ctx.finish_output(r.text, true);
        out.text = self.fenced(&out.text);
        out.images = r
            .images
            .into_iter()
            .map(|(media_type, data_b64)| agenium_tools::ImageData {
                media_type,
                data_b64,
                label: format!("from {}", self.name),
            })
            .collect();
        Ok(out)
    }
}
