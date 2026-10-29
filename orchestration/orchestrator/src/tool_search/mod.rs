//! Finding MCP tools instead of listing them all (ADR-139). Every listed tool costs its definition in every request
//! and blurs the model's choice past a few dozen; Anthropic's API defers tools behind a search for that reason. Here it
//! is done for every provider and CLI: past `[tool_search]` thresholds a run lists only the MCP tools its agent used
//! most, with `tool_search` (words, meaning and the judge rank the catalogue) and `call_tool` (runs what was found).
//! The run's tool list never changes, so the prompt-cache prefix holds. A server that may change its tools during the
//! run (it declares `tools.listChanged`) gets the pair too, small as it is: a server that offers its own search over a
//! few tools and falls back to listing a hundred (its search out of quota) is followed — the new tools are searched,
//! not added to the prompt.

mod call;
mod index;
mod jev;
mod search;

use std::sync::{Arc, Mutex};

use agenium_core::AgentId;
use agenium_mcp::Catalog;
use agenium_tools::Tool;

use crate::app::App;

/// What a run gets of its MCP tools.
pub(crate) struct McpOffer {
    pub tools: Vec<Arc<dyn Tool>>,
    /// Lines for the run's capabilities (what is listed, what is found by search).
    pub lines: Vec<String>,
}

/// Names of found-by-search tools listed in the prompt at most (beyond this only their number and servers).
const NAME_LIST: usize = 200;

/// An estimate of the tokens a tool's definition takes in a request.
fn tokens(t: &dyn Tool) -> usize {
    (t.name().len() + t.description().len() + t.parameters().to_string().len()) / 4
}

impl App {
    /// The tools `tool_search` returns for `query` from `catalog`, best first, by words alone or with their meaning
    /// (the judge as `[tool_search] judge` says) — ADR-139's measurement.
    pub async fn find_mcp_tools(
        self: &Arc<Self>,
        catalog: Arc<Catalog>,
        query: &str,
        limit: usize,
        meaning: bool,
    ) -> Vec<String> {
        let s = search::ToolSearch {
            app: self.clone(),
            catalog,
            ticket: None,
            index: Mutex::new(None),
            meaning,
        };
        let idx = s.index().await;
        s.find(&idx, query, None, limit)
            .await
            .into_iter()
            .map(|i| idx.tools[i].mcp_name().to_owned())
            .collect()
    }
}

/// The run's MCP tools: all listed, or (past the thresholds) the most used listed and the rest behind `tool_search`.
pub(crate) async fn offer(
    app: &Arc<App>,
    agent: AgentId,
    catalog: Arc<Catalog>,
    ticket: Option<String>,
) -> McpOffer {
    let all = catalog.listed().await;
    let cfg = app.config().tool_search;
    let size: usize = all.iter().map(|t| tokens(t.as_ref())).sum();
    let defer = cfg.defers(all.len(), size);
    let pair = |catalog: &Arc<Catalog>| -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(search::ToolSearch {
                app: app.clone(),
                catalog: catalog.clone(),
                ticket: ticket.clone(),
                index: Mutex::new(None),
                meaning: true,
            }),
            Arc::new(call::CallTool {
                catalog: catalog.clone(),
            }),
        ]
    };
    if !defer {
        let mut tools: Vec<Arc<dyn Tool>> = all.into_iter().map(|t| t as Arc<dyn Tool>).collect();
        let mut lines = Vec::new();
        if catalog.may_change() {
            tools.extend(pair(&catalog));
            lines.push(
                "An MCP server may change its tools during the run: tools it adds are found with tool_search and \
                 called with call_tool."
                    .to_owned(),
            );
        }
        return McpOffer { tools, lines };
    }
    let used: Vec<String> = app
        .store()
        .read(|c| agenium_store::core::runs::mcp_tools_used(c, agent, cfg.keep_used))
        .unwrap_or_default();
    let (listed, found): (Vec<_>, Vec<_>) = all
        .into_iter()
        .partition(|t| used.iter().any(|u| u == t.name()));
    let mut line = format!(
        "{} more MCP tools are not listed: find them with tool_search (say what you want to do) and call them with \
         call_tool.",
        found.len()
    );
    if found.len() <= NAME_LIST {
        let mut by_server: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
        for t in &found {
            by_server.entry(t.server()).or_default().push(t.mcp_name());
        }
        for (server, names) in by_server {
            line.push_str(&format!(" {server}: {}.", names.join(", ")));
        }
    }
    let mut tools: Vec<Arc<dyn Tool>> = listed.into_iter().map(|t| t as Arc<dyn Tool>).collect();
    tools.extend(pair(&catalog));
    McpOffer {
        tools,
        lines: vec![line],
    }
}
