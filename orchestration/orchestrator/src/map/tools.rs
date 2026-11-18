//! The map tools of a project's agents (ADR-166): `map_add` puts what the agent traced or read on the map once
//! Agenium has checked it, `map_query` asks the map, `map_retract` takes back what no longer holds (only when its
//! evidence shows it). Code is checked in the run's own tree; a page is fetched by Agenium itself.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use agenium_core::{ProjectId, RunId, TicketKey, TicketStatus};
use agenium_store::core::map::{self as store, MapLink, MapNode};
use agenium_tools::{Tool, ToolCtx, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};

use super::refs::{link_kind, Ref};
use super::verify::{self, Evidence};
use super::{line_text, search, CONFIRMED, STALE, TENTATIVE};
use crate::App;

/// What one run may add: enough for a new project's structure, not a dump of everything.
const MAX_NODES: usize = 80;
const MAX_LINKS: usize = 160;

/// Who adds to the map.
#[derive(Clone)]
pub struct Who {
    pub project: ProjectId,
    pub ticket: Option<TicketKey>,
    pub agent: String,
    pub run: RunId,
}

struct Shared {
    app: Arc<App>,
    who: Who,
    nodes: AtomicUsize,
    links: AtomicUsize,
    /// Pages fetched in this run, by address: their text, or why they could not be read.
    pages: Mutex<HashMap<String, Result<String, String>>>,
}

/// The three map tools of a run.
pub fn tools(app: Arc<App>, who: Who) -> Vec<Arc<dyn Tool>> {
    let s = Arc::new(Shared {
        app,
        who,
        nodes: AtomicUsize::new(0),
        links: AtomicUsize::new(0),
        pages: Mutex::default(),
    });
    vec![
        Arc::new(MapAdd(s.clone())),
        Arc::new(MapQuery(s.clone())),
        Arc::new(MapRetract(s)),
    ]
}

fn err(m: impl std::fmt::Display) -> ToolError {
    ToolError::new("invalid_arguments", m.to_string())
}

impl Shared {
    fn project(&self) -> String {
        self.who.project.to_string()
    }

    /// Tentative while the run's ticket is open (confirmed when it closes), confirmed without a ticket.
    fn status(&self) -> &'static str {
        match &self.who.ticket {
            Some(k) => match self.app.ticket(self.who.project, k) {
                Ok(t) if t.status == TicketStatus::Closed => CONFIRMED,
                _ => TENTATIVE,
            },
            None => CONFIRMED,
        }
    }

    /// The text of the page at `url`, read by Agenium (the same way and under the same rules as `fetch`).
    async fn page(&self, ctx: &ToolCtx, url: &str) -> Result<String, String> {
        if let Some(r) = self
            .pages
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(url)
        {
            return r.clone();
        }
        let r = match agenium_tools::FetchTool
            .call(ctx, json!({ "url": url }))
            .await
        {
            Ok(o) => Ok(match &o.full {
                Some(rel) => std::fs::read_to_string(ctx.root.join(rel)).unwrap_or(o.text),
                None => o.text,
            }),
            Err(e) => Err(format!("Agenium could not read {url}: {}", e.message)),
        };
        self.pages
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(url.to_owned(), r.clone());
        r
    }

    /// Whether `ev` names every one of `names` (a page, or the cited lines of code).
    async fn shows(&self, ctx: &ToolCtx, ev: &Evidence, names: &[&str]) -> Result<(), String> {
        let text = match ev {
            Evidence::Web { url } => self.page(ctx, url).await?,
            Evidence::Code { file, start, end } => verify::lines(&ctx.root, file, *start, *end)?,
        };
        match names.iter().find(|n| !verify::names(&text, n, false)) {
            None => Ok(()),
            Some(n) => Err(format!(
                "{} does not mention {n:?}: name it as the source does, or cite a source that names it",
                ev.text()
            )),
        }
    }

    fn node(&self, key: &str) -> Option<MapNode> {
        let p = self.project();
        self.app
            .store()
            .read(|c| store::node(c, &p, key))
            .ok()
            .flatten()
    }

    /// Checks node `r` (a code node by its file and declaration; anything else by `ev`, which must name it) and
    /// keeps it. An existing confirmed node stays confirmed.
    async fn put_node(
        &self,
        ctx: &ToolCtx,
        r: &Ref,
        summary: &str,
        ev: Option<&Evidence>,
    ) -> Result<(), String> {
        let (lines, evidence) = if r.is_code() {
            (Some(verify::locate(&ctx.root, r)?), json!({}))
        } else {
            let ev = ev.ok_or_else(|| {
                format!(
                    "{} needs evidence: the page (url) or the code (file, line) that names it",
                    r.key()
                )
            })?;
            self.shows(ctx, ev, &[&r.name]).await?;
            (None, ev.json())
        };
        let old = self.node(&r.key());
        if old.is_none() && self.nodes.fetch_add(1, Ordering::Relaxed) >= MAX_NODES {
            return Err(format!(
                "this run already added {MAX_NODES} nodes: add the parts that matter, not every one"
            ));
        }
        let now = agenium_core::now_iso();
        let keep = old.as_ref().is_some_and(|o| o.status == CONFIRMED);
        let n = MapNode {
            key: r.key(),
            kind: r.kind.clone(),
            name: r.name.clone(),
            file: r.file.clone(),
            line_start: lines.map(|l| l.0),
            line_end: lines.map(|l| l.1),
            summary: if summary.trim().is_empty() {
                old.as_ref().map(|o| o.summary.clone()).unwrap_or_default()
            } else {
                summary.trim().chars().take(300).collect()
            },
            evidence,
            status: if keep { CONFIRMED } else { self.status() }.into(),
            ticket: if keep {
                old.as_ref().and_then(|o| o.ticket.clone())
            } else {
                self.who.ticket.as_ref().map(|t| t.to_string())
            },
            agent: old
                .as_ref()
                .map_or(self.who.agent.clone(), |o| o.agent.clone()),
            run: self.who.run.to_string(),
            created_at: old.as_ref().map_or(now.clone(), |o| o.created_at.clone()),
            checked_at: now,
        };
        let p = self.project();
        self.app
            .store()
            .commit(None, "system", |w| store::put_node(w.tx, &p, &n))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn put_link(&self, ctx: &ToolCtx, l: &Value) -> Result<String, String> {
        let s = |k: &str| l[k].as_str().unwrap_or_default();
        let from = Ref::parse(s("from")).map_err(|e| e.to_string())?;
        let to = Ref::parse(s("to")).map_err(|e| e.to_string())?;
        let kind = link_kind(s("type")).map_err(|e| e.to_string())?;
        let ev = Evidence::parse(&l["evidence"])?;
        // Code that does not exist is said first: it tells more than the line that does not name it.
        for r in [&from, &to].into_iter().filter(|r| r.is_code()) {
            verify::locate(&ctx.root, r)?;
        }
        match &ev {
            Evidence::Code { .. } => verify::code_link(&ctx.root, &ev, &from, &to)?,
            Evidence::Web { .. } if from.is_code() || to.is_code() => {
                return Err("a link of code is shown by code: cite its file and line".into())
            }
            Evidence::Web { .. } => self.shows(ctx, &ev, &[&from.name, &to.name]).await?,
        }
        // Its ends are put on the map too: code by its declaration, anything else by this evidence.
        for r in [&from, &to] {
            if self.node(&r.key()).is_none() {
                let own = (!r.is_code()).then_some(&ev);
                self.put_node(ctx, r, "", own).await?;
            }
        }
        if self.links.fetch_add(1, Ordering::Relaxed) >= MAX_LINKS {
            return Err(format!("this run already added {MAX_LINKS} links"));
        }
        let now = agenium_core::now_iso();
        let link = MapLink {
            id: 0,
            src: from.key(),
            kind,
            dst: to.key(),
            evidence: ev.json(),
            status: self.status().into(),
            ticket: self.who.ticket.as_ref().map(|t| t.to_string()),
            agent: self.who.agent.clone(),
            run: self.who.run.to_string(),
            created_at: now.clone(),
            checked_at: now,
        };
        let p = self.project();
        self.app
            .store()
            .commit(None, "system", |w| store::put_link(w.tx, &p, &link))
            .map_err(|e| e.to_string())?;
        Ok(line_text(&link))
    }
}

struct MapAdd(Arc<Shared>);

#[async_trait]
impl Tool for MapAdd {
    fn name(&self) -> &str {
        "map_add"
    }
    fn description(&self) -> &str {
        "Add to the project's map what you traced or read, so later work (yours and other agents') starts from it: \
         nodes are kind:name (file:src/app.py, symbol:src/app.py#load_config, or any kind for what is not code: \
         service:billing, product:Claude Desktop, domain:api.anthropic.com, endpoint:POST /v1/messages); links join two \
         with a type (calls, imports, reads, writes, requests, hosted_at, part_of, depends_on…) and their evidence: \
         {\"file\", \"line\"} or {\"file\", \"lines\": [from, to]} of the source's own code that names the target, or \
         {\"url\"} of a page that names both ends. Agenium checks each one before it is kept (the code in your tree, \
         the page by fetching it) and says why it refused one. Add what you verified, once you know it holds."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "nodes": { "type": "array", "items": { "type": "object", "properties": {
                    "name": { "type": "string", "description": "kind:name" },
                    "summary": { "type": "string", "description": "What it is or does, one or two sentences" },
                    "evidence": { "type": "object", "description": "For a node that is not code: {\"url\"} or {\"file\", \"line\"} that names it" }
                }, "required": ["name"] } },
                "links": { "type": "array", "items": { "type": "object", "properties": {
                    "from": { "type": "string" },
                    "type": { "type": "string" },
                    "to": { "type": "string" },
                    "evidence": { "type": "object" }
                }, "required": ["from", "type", "to", "evidence"] } }
            }
        })
    }
    async fn call(&self, ctx: &ToolCtx, args: Value) -> Result<ToolOutput, ToolError> {
        let s = &self.0;
        let (nodes, links) = (
            args["nodes"].as_array().cloned().unwrap_or_default(),
            args["links"].as_array().cloned().unwrap_or_default(),
        );
        if nodes.is_empty() && links.is_empty() {
            return Err(err("map_add needs nodes or links"));
        }
        let (mut added, mut refused) = (Vec::new(), Vec::new());
        for n in &nodes {
            let name = n["name"].as_str().unwrap_or_default();
            let r = match Ref::parse(name) {
                Ok(r) => r,
                Err(e) => {
                    refused.push(format!("{name}: {e}"));
                    continue;
                }
            };
            let ev = match n.get("evidence").filter(|e| !e.is_null()) {
                Some(e) => match Evidence::parse(e) {
                    Ok(e) => Some(e),
                    Err(e) => {
                        refused.push(format!("{name}: {e}"));
                        continue;
                    }
                },
                None => None,
            };
            match s
                .put_node(
                    ctx,
                    &r,
                    n["summary"].as_str().unwrap_or_default(),
                    ev.as_ref(),
                )
                .await
            {
                Ok(()) => added.push(r.key()),
                Err(e) => refused.push(format!("{name}: {e}")),
            }
        }
        for l in &links {
            match s.put_link(ctx, l).await {
                Ok(t) => added.push(t),
                Err(e) => refused.push(format!(
                    "{} --{}--> {}: {e}",
                    l["from"].as_str().unwrap_or("?"),
                    l["type"].as_str().unwrap_or("?"),
                    l["to"].as_str().unwrap_or("?")
                )),
            }
        }
        let status = s.status();
        let mut out = format!(
            "Kept {} ({}):\n{}",
            added.len(),
            if status == TENTATIVE {
                "tentative until your ticket is closed, then checked again on the main branch"
            } else {
                "confirmed"
            },
            added.join("\n")
        );
        if !refused.is_empty() {
            out.push_str(&format!(
                "\n\nRefused {} (fix and add again):\n{}",
                refused.len(),
                refused.join("\n")
            ));
        }
        Ok(ToolOutput::text(out))
    }
}

struct MapQuery(Arc<Shared>);

#[async_trait]
impl Tool for MapQuery {
    fn name(&self) -> &str {
        "map_query"
    }
    fn description(&self) -> &str {
        "Ask the project's map how its parts connect before you trace them from scratch: find nodes (find, with kind: \
         to keep one kind), the links of a node (node, direction in/out/both, type, depth up to 3), what depends on it \
         (node with impact), or the chain from one node to another (node with path_to). Each link comes with its \
         evidence (file:line or a page): read it before you rely on it; a stale link's evidence no longer matched."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "find": { "type": "string" },
                "node": { "type": "string", "description": "kind:name" },
                "direction": { "type": "string", "enum": ["in", "out", "both"] },
                "type": { "type": "string" },
                "depth": { "type": "integer", "minimum": 1, "maximum": 3 },
                "impact": { "type": "boolean" },
                "path_to": { "type": "string" }
            }
        })
    }
    fn read_only(&self) -> bool {
        true
    }
    async fn call(&self, _ctx: &ToolCtx, a: Value) -> Result<ToolOutput, ToolError> {
        let s = &self.0;
        let w = s
            .app
            .map_whole(s.who.project)
            .map_err(|e| ToolError::new("map", e.to_string()))?;
        if w.nodes.is_empty() {
            return Ok(ToolOutput::text(
                "The project's map is empty: add what you trace with map_add.",
            ));
        }
        if let Some(q) = a["find"].as_str() {
            let hits = search(&w, q, 30);
            if hits.is_empty() {
                return Ok(ToolOutput::text(format!("No node matches {q:?}.")));
            }
            return Ok(ToolOutput::text(
                hits.iter()
                    .map(|n| {
                        let links = w
                            .links
                            .iter()
                            .filter(|l| l.src == n.key || l.dst == n.key)
                            .count();
                        format!(
                            "{}  ({} links{}){}",
                            n.key,
                            links,
                            if n.status == CONFIRMED {
                                String::new()
                            } else {
                                format!(", {}", n.status)
                            },
                            if n.summary.is_empty() {
                                String::new()
                            } else {
                                format!(": {}", n.summary)
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ));
        }
        let key = Ref::parse(a["node"].as_str().unwrap_or_default())
            .map_err(err)?
            .key();
        let Some(n) = w.nodes.get(&key) else {
            let near = search(&w, key.rsplit([':', '#', '/']).next().unwrap_or(&key), 5);
            return Err(ToolError::new(
                "not_found",
                format!(
                    "{key} is not on the map{}",
                    if near.is_empty() {
                        String::new()
                    } else {
                        format!(
                            "; near: {}",
                            near.iter()
                                .map(|n| n.key.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                ),
            ));
        };
        if let Some(to) = a["path_to"].as_str() {
            let to = Ref::parse(to).map_err(err)?.key();
            return Ok(ToolOutput::text(match w.path(&key, &to) {
                None => format!("No chain of links joins {key} and {to} on the map."),
                Some((ids, directed)) => format!(
                    "{}\n{}",
                    if directed {
                        "Following the links:"
                    } else {
                        "Only with links taken either way:"
                    },
                    ids.iter()
                        .filter_map(|id| w.links.iter().find(|l| l.id == *id))
                        .map(line_text)
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            }));
        }
        let depth = a["depth"].as_u64().unwrap_or(1).clamp(1, 3) as usize;
        let impact = a["impact"].as_bool().unwrap_or(false);
        let dir = if impact {
            "in"
        } else {
            a["direction"].as_str().unwrap_or("both")
        };
        let kind = a["type"].as_str().map(|t| t.trim().to_lowercase());
        let mut lines = vec![format!(
            "{} ({}{}){}",
            n.key,
            n.status,
            n.file
                .as_ref()
                .map(|f| match (n.line_start, n.line_end) {
                    (Some(a), Some(b)) => format!(", {f}:{a}-{b}"),
                    _ => format!(", {f}"),
                })
                .unwrap_or_default(),
            if n.summary.is_empty() {
                String::new()
            } else {
                format!(": {}", n.summary)
            }
        )];
        let mut front = vec![key.clone()];
        let mut seen = std::collections::HashSet::from([key.clone()]);
        let mut shown = std::collections::HashSet::new();
        for _ in 0..depth {
            let mut next = Vec::new();
            for k in &front {
                for l in w
                    .links
                    .iter()
                    .filter(|l| kind.as_deref().is_none_or(|t| l.kind == t))
                {
                    let other = match dir {
                        "out" if &l.src == k => &l.dst,
                        "in" if &l.dst == k => &l.src,
                        "both" if &l.src == k => &l.dst,
                        "both" if &l.dst == k => &l.src,
                        _ => continue,
                    };
                    if lines.len() < 80 && shown.insert(l.id) {
                        lines.push(line_text(l));
                    }
                    if seen.insert(other.clone()) {
                        next.push(other.clone());
                    }
                }
            }
            front = next;
        }
        if lines.len() == 1 {
            lines.push("(no links of that kind)".into());
        }
        Ok(ToolOutput::text(lines.join("\n")))
    }
}

struct MapRetract(Arc<Shared>);

#[async_trait]
impl Tool for MapRetract {
    fn name(&self) -> &str {
        "map_retract"
    }
    fn description(&self) -> &str {
        "Take a link or a node off the project's map when it is wrong or no longer true (your change removed that \
         call, the page moved). What your own ticket added goes at once; anything else only once Agenium sees its \
         evidence no longer holds (the code in your tree, the page fetched again). Give the reason."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "link": { "type": "object", "properties": {
                    "from": { "type": "string" }, "type": { "type": "string" }, "to": { "type": "string" } } },
                "node": { "type": "string" },
                "reason": { "type": "string" }
            },
            "required": ["reason"]
        })
    }
    async fn call(&self, ctx: &ToolCtx, a: Value) -> Result<ToolOutput, ToolError> {
        let s = &self.0;
        let p = s.project();
        let mine = |ticket: &Option<String>, status: &str| {
            status == TENTATIVE
                && ticket.as_deref() == s.who.ticket.as_ref().map(|t| t.to_string()).as_deref()
        };
        if let Some(l) = a.get("link").filter(|l| l.is_object()) {
            let g = |k: &str| l[k].as_str().unwrap_or_default();
            let (from, to) = (
                Ref::parse(g("from")).map_err(err)?,
                Ref::parse(g("to")).map_err(err)?,
            );
            let kind = link_kind(g("type")).map_err(err)?;
            let found = s
                .app
                .store()
                .read(|c| store::links_of(c, &p, &from.key()))
                .map_err(|e| ToolError::new("map", e.to_string()))?
                .into_iter()
                .find(|x| x.src == from.key() && x.dst == to.key() && x.kind == kind)
                .ok_or_else(|| ToolError::new("not_found", "no such link on the map"))?;
            if !mine(&found.ticket, &found.status) && found.status != STALE {
                let ev = Evidence::parse(&found.evidence).map_err(err)?;
                let holds = match &ev {
                    Evidence::Code { .. } => verify::code_link(&ctx.root, &ev, &from, &to).is_ok(),
                    Evidence::Web { .. } => {
                        s.shows(ctx, &ev, &[&from.name, &to.name]).await.is_ok()
                    }
                };
                if holds {
                    return Err(ToolError::new(
                        "still_holds",
                        format!(
                            "its evidence {} still shows it; change what it cites first, or leave it",
                            ev.text()
                        ),
                    ));
                }
            }
            s.app
                .store()
                .commit(None, "system", |w| store::remove_link(w.tx, &p, found.id))
                .map_err(|e| ToolError::new("map", e.to_string()))?;
            return Ok(ToolOutput::text(format!(
                "Taken off: {}",
                line_text(&found)
            )));
        }
        let r = Ref::parse(a["node"].as_str().unwrap_or_default()).map_err(err)?;
        let n = s
            .node(&r.key())
            .ok_or_else(|| ToolError::new("not_found", format!("{} is not on the map", r.key())))?;
        if !mine(&n.ticket, &n.status) && n.status != STALE {
            let holds = if r.is_code() {
                verify::locate(&ctx.root, &r).is_ok()
            } else {
                match Evidence::parse(&n.evidence) {
                    Ok(ev) => s.shows(ctx, &ev, &[&r.name]).await.is_ok(),
                    Err(_) => false,
                }
            };
            if holds {
                return Err(ToolError::new(
                    "still_holds",
                    format!("{} still exists by its evidence; it stays", r.key()),
                ));
            }
        }
        s.app
            .store()
            .commit(None, "system", |w| store::remove_node(w.tx, &p, &r.key()))
            .map_err(|e| ToolError::new("map", e.to_string()))?;
        Ok(ToolOutput::text(format!(
            "Taken off: {} and its links",
            r.key()
        )))
    }
}
