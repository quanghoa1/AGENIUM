//! The project map (ADR-166): what the project's agents found out about how its parts connect (code that calls,
//! imports, reads; a product hosted at a domain, an app that requests an API), each piece with evidence Agenium
//! checked. One map per project, kept by Agenium (not in the repository: only the map tools change it). Agents add to
//! it and ask it (`tools`); the user only looks at it. What a ticket added stays tentative until the ticket is closed
//! (checked again on the main branch then), and is dropped if the ticket is cancelled; later changes that break a
//! piece of code evidence make it stale.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use agenium_core::{ProjectId, Ticket, TicketStatus};
use agenium_store::core::map::{self as store, MapLink, MapNode};
use serde::Serialize;

use crate::{App, Result};

pub mod refs;
pub mod tools;
pub mod verify;

use refs::Ref;
use verify::Evidence;

pub const TENTATIVE: &str = "tentative";
pub const CONFIRMED: &str = "confirmed";
pub const STALE: &str = "stale";

/// Nodes a view of the map shows at most.
const VIEW_NODES: usize = 200;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct NodeView {
    pub key: String,
    pub kind: String,
    pub name: String,
    pub file: Option<String>,
    pub lines: Option<(u32, u32)>,
    pub summary: String,
    pub status: String,
    pub ticket: Option<String>,
    pub agent: String,
    pub evidence: String,
    pub added_at: String,
    pub checked_at: String,
    /// Links from and to it in the whole map.
    pub degree: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LinkView {
    pub id: i64,
    pub src: String,
    pub kind: String,
    pub dst: String,
    pub status: String,
    pub evidence: String,
    pub ticket: Option<String>,
    pub agent: String,
    pub added_at: String,
}

/// A part of the map to draw.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct MapGraph {
    pub nodes: Vec<NodeView>,
    pub links: Vec<LinkView>,
    /// Nodes left out of a view over `VIEW_NODES`.
    pub more: usize,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct MapStats {
    pub nodes: usize,
    pub links: usize,
    pub stale: usize,
    pub tentative: usize,
    pub kinds: Vec<(String, usize)>,
}

fn evidence_text(v: &serde_json::Value) -> String {
    Evidence::parse(v).map(|e| e.text()).unwrap_or_default()
}

fn node_view(n: &MapNode, degree: usize) -> NodeView {
    NodeView {
        key: n.key.clone(),
        kind: n.kind.clone(),
        name: n.name.clone(),
        file: n.file.clone(),
        lines: n.line_start.zip(n.line_end),
        summary: n.summary.clone(),
        status: n.status.clone(),
        ticket: n.ticket.clone(),
        agent: n.agent.clone(),
        evidence: evidence_text(&n.evidence),
        added_at: n.created_at.clone(),
        checked_at: n.checked_at.clone(),
        degree,
    }
}

fn link_view(l: &MapLink) -> LinkView {
    LinkView {
        id: l.id,
        src: l.src.clone(),
        kind: l.kind.clone(),
        dst: l.dst.clone(),
        status: l.status.clone(),
        evidence: evidence_text(&l.evidence),
        ticket: l.ticket.clone(),
        agent: l.agent.clone(),
        added_at: l.created_at.clone(),
    }
}

/// The whole map of a project, read once.
pub(crate) struct Whole {
    pub nodes: HashMap<String, MapNode>,
    pub links: Vec<MapLink>,
}

impl Whole {
    fn degree(&self) -> HashMap<&str, usize> {
        let mut d: HashMap<&str, usize> = HashMap::new();
        for l in &self.links {
            *d.entry(l.src.as_str()).or_default() += 1;
            *d.entry(l.dst.as_str()).or_default() += 1;
        }
        d
    }

    fn graph(&self, keys: &[String], extra_links: Option<&[i64]>) -> MapGraph {
        let deg = self.degree();
        let keep: HashSet<&str> = keys.iter().take(VIEW_NODES).map(String::as_str).collect();
        MapGraph {
            nodes: keys
                .iter()
                .take(VIEW_NODES)
                .filter_map(|k| self.nodes.get(k))
                .map(|n| node_view(n, deg.get(n.key.as_str()).copied().unwrap_or(0)))
                .collect(),
            links: self
                .links
                .iter()
                .filter(|l| match extra_links {
                    Some(ids) => ids.contains(&l.id),
                    None => keep.contains(l.src.as_str()) && keep.contains(l.dst.as_str()),
                })
                .map(link_view)
                .collect(),
            more: keys.len().saturating_sub(VIEW_NODES),
        }
    }

    /// The keys within `depth` links of `key` (either way), nearest first.
    pub fn around(&self, key: &str, depth: usize) -> Vec<String> {
        let mut seen: Vec<String> = vec![key.to_owned()];
        let mut set: HashSet<String> = seen.iter().cloned().collect();
        let mut front = vec![key.to_owned()];
        for _ in 0..depth {
            let mut next = Vec::new();
            for k in &front {
                for l in self.links.iter().filter(|l| &l.src == k || &l.dst == k) {
                    let o = if &l.src == k { &l.dst } else { &l.src };
                    if set.insert(o.clone()) {
                        seen.push(o.clone());
                        next.push(o.clone());
                    }
                }
            }
            front = next;
        }
        seen
    }

    /// The shortest chain of links from `from` to `to` following their direction, else either way (`false`).
    pub fn path(&self, from: &str, to: &str) -> Option<(Vec<i64>, bool)> {
        // Each node's links, once: a search then reads only the links of the nodes it reaches.
        let mut adj: HashMap<&str, Vec<&MapLink>> = HashMap::new();
        for l in &self.links {
            adj.entry(l.src.as_str()).or_default().push(l);
            if l.dst != l.src {
                adj.entry(l.dst.as_str()).or_default().push(l);
            }
        }
        for directed in [true, false] {
            let mut prev: HashMap<&str, (i64, &str)> = HashMap::new();
            let mut q = VecDeque::from([from]);
            let mut seen: HashSet<&str> = HashSet::from([from]);
            while let Some(k) = q.pop_front() {
                if k == to {
                    let mut ids = Vec::new();
                    let mut at = to;
                    while at != from {
                        let (id, p) = prev[at];
                        ids.push(id);
                        at = p;
                    }
                    ids.reverse();
                    return Some((ids, directed));
                }
                for l in adj.get(k).into_iter().flatten() {
                    let next = if l.src == k {
                        Some(l.dst.as_str())
                    } else if !directed && l.dst == k {
                        Some(l.src.as_str())
                    } else {
                        None
                    };
                    if let Some(n) = next {
                        if seen.insert(n) {
                            prev.insert(n, (l.id, k));
                            q.push_back(n);
                        }
                    }
                }
            }
        }
        None
    }
}

/// Nodes a search matches: `kind:` before the words keeps one kind (`symbol:load`, `domain:anthropic`).
pub(crate) fn search<'a>(w: &'a Whole, q: &str, limit: usize) -> Vec<&'a MapNode> {
    let q = q.trim().to_lowercase();
    let (kind, words) = match q.split_once(':') {
        Some((k, rest)) if w.nodes.values().any(|n| n.kind == k) => {
            (Some(k.to_owned()), rest.trim().to_owned())
        }
        _ => (None, q.clone()),
    };
    let mut hits: Vec<(usize, &MapNode)> = w
        .nodes
        .values()
        .filter(|n| kind.as_deref().is_none_or(|k| n.kind == k))
        .filter_map(|n| {
            let name = n.name.to_lowercase();
            let rank = if words.is_empty() || name == words {
                0
            } else if name.starts_with(&words)
                || name.rsplit(['#', '/']).next() == Some(words.as_str())
            {
                1
            } else if name.contains(&words) {
                2
            } else if n.summary.to_lowercase().contains(&words) {
                3
            } else {
                return None;
            };
            Some((rank, n))
        })
        .collect();
    hits.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.key.cmp(&b.1.key)));
    hits.into_iter().take(limit).map(|(_, n)| n).collect()
}

impl App {
    pub(crate) fn map_whole(&self, project: ProjectId) -> Result<Whole> {
        let p = project.to_string();
        let (nodes, links) = self
            .store()
            .read(|c| Ok((store::nodes(c, &p)?, store::links(c, &p)?)))?;
        Ok(Whole {
            nodes: nodes.into_iter().map(|n| (n.key.clone(), n)).collect(),
            links,
        })
    }

    pub fn map_stats(&self, project: ProjectId) -> Result<MapStats> {
        let w = self.map_whole(project)?;
        let mut kinds: HashMap<String, usize> = HashMap::new();
        for n in w.nodes.values() {
            *kinds.entry(n.kind.clone()).or_default() += 1;
        }
        let mut kinds: Vec<_> = kinds.into_iter().collect();
        kinds.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let all = || {
            w.nodes
                .values()
                .map(|n| n.status.as_str())
                .chain(w.links.iter().map(|l| l.status.as_str()))
        };
        Ok(MapStats {
            nodes: w.nodes.len(),
            links: w.links.len(),
            stale: all().filter(|s| *s == STALE).count(),
            tentative: all().filter(|s| *s == TENTATIVE).count(),
            kinds,
        })
    }

    /// Nodes the user's search finds (the Map page's search box).
    pub fn map_search(&self, project: ProjectId, q: &str) -> Result<Vec<NodeView>> {
        let w = self.map_whole(project)?;
        let deg = w.degree();
        Ok(search(&w, q, 30)
            .into_iter()
            .map(|n| node_view(n, deg.get(n.key.as_str()).copied().unwrap_or(0)))
            .collect())
    }

    /// `key` and what lies within `depth` links of it; without a key the most connected part of the map.
    pub fn map_around(
        &self,
        project: ProjectId,
        key: Option<&str>,
        depth: usize,
    ) -> Result<MapGraph> {
        let w = self.map_whole(project)?;
        let keys = match key.filter(|k| w.nodes.contains_key(*k)) {
            Some(k) => w.around(k, depth.clamp(1, 3)),
            None => {
                let deg = w.degree();
                let mut all: Vec<&String> = w.nodes.keys().collect();
                all.sort_by(|a, b| {
                    deg.get(b.as_str())
                        .unwrap_or(&0)
                        .cmp(deg.get(a.as_str()).unwrap_or(&0))
                        .then(a.cmp(b))
                });
                all.into_iter().cloned().collect()
            }
        };
        Ok(w.graph(&keys, None))
    }

    /// The shortest chain of links from `from` to `to` (following their direction when one exists).
    pub fn map_path(&self, project: ProjectId, from: &str, to: &str) -> Result<Option<MapGraph>> {
        let w = self.map_whole(project)?;
        let Some((ids, _)) = w.path(from, to) else {
            return Ok(None);
        };
        let mut keys = vec![from.to_owned()];
        for id in &ids {
            if let Some(l) = w.links.iter().find(|l| l.id == *id) {
                for k in [&l.src, &l.dst] {
                    if !keys.contains(k) {
                        keys.push(k.clone());
                    }
                }
            }
        }
        Ok(Some(w.graph(&keys, Some(&ids))))
    }

    /// The lines a piece of code evidence cites (`file:12` or `file:12-18`), as the project's main branch has them,
    /// with two lines around: what the Map page shows beside a link.
    pub fn map_code(&self, project: ProjectId, evidence: &str) -> Result<String> {
        let p = self.project(project)?;
        let (file, range) = evidence
            .rsplit_once(':')
            .ok_or_else(|| crate::OrchError::Invalid(format!("{evidence:?} is not file:line")))?;
        let (a, b) = match range.split_once('-') {
            Some((a, b)) => (a.parse().unwrap_or(0), b.parse().unwrap_or(0)),
            None => {
                let l = range.parse().unwrap_or(0);
                (l, l)
            }
        };
        let root = crate::project_create::checkout_dir(Path::new(&p.path));
        let text = std::fs::read_to_string(root.join(file))
            .ok()
            .filter(|_| agenium_tools::paths::confined(&root.join(file), &root))
            .ok_or_else(|| crate::OrchError::NotFound(file.to_owned()))?;
        let lines: Vec<&str> = text.lines().collect();
        let (from, to) = (
            (a as usize).saturating_sub(3),
            (b as usize + 2).min(lines.len()),
        );
        Ok(lines
            .iter()
            .enumerate()
            .take(to)
            .skip(from)
            .map(|(i, l)| {
                format!(
                    "{}{:>5}  {l}",
                    if i + 1 >= a as usize && i < b as usize {
                        ">"
                    } else {
                        " "
                    },
                    i + 1
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// What a ticket's work order shows of the map: the links of the nodes its text or scope names.
    pub fn map_near(&self, project: ProjectId, t: &Ticket) -> String {
        let Ok(w) = self.map_whole(project) else {
            return String::new();
        };
        if w.links.is_empty() {
            return String::new();
        }
        let text = format!("{}\n{}", t.title, t.description).to_lowercase();
        let named = |n: &MapNode| {
            let name = n.name.to_lowercase();
            let short = name.rsplit(['#', '/']).next().unwrap_or(&name).to_owned();
            (n.file.as_deref().is_some_and(|f| {
                text.contains(&f.to_lowercase())
                    || t.scope_paths
                        .iter()
                        .any(|s| f.starts_with(s.trim_end_matches(['*', '/'])))
            })) || (short.len() >= 5 && verify::names(&text, &short, true))
        };
        let mut seeds: Vec<&MapNode> = w.nodes.values().filter(|n| named(n)).collect();
        seeds.sort_by(|a, b| a.key.cmp(&b.key));
        let mut lines = Vec::new();
        let mut shown = HashSet::new();
        for s in seeds.iter().take(8) {
            for l in w.links.iter().filter(|l| l.src == s.key || l.dst == s.key) {
                if lines.len() >= 24 || !shown.insert(l.id) {
                    continue;
                }
                lines.push(line_text(l));
            }
        }
        if lines.is_empty() {
            return String::new();
        }
        format!(
            "## On the project map (links agents traced, with evidence; read the code before you rely on one, and ask \
             map_query for more)\n{}\n\n",
            lines.join("\n")
        )
    }

    /// Settles what tickets added (ADR-166): a closed ticket's entries are checked again on the main branch and
    /// confirmed (or stale), a cancelled ticket's are dropped; after a ticket closed, every confirmed piece of code
    /// evidence is checked again (and follows code that moved). Cheap: a few file reads per link.
    pub fn map_settle(&self, project: ProjectId) -> Result<()> {
        let p = project.to_string();
        let tickets = self.store().read(|c| store::tentative_tickets(c, &p))?;
        let mut closed = false;
        // Read once: each ticket's entries are its own, and confirming an entry twice changes nothing.
        let mut whole = None;
        for key in tickets {
            let Ok(k) = key.parse() else { continue };
            let Ok(t) = self.ticket(project, &k) else {
                continue;
            };
            match t.status {
                TicketStatus::Cancelled => {
                    self.store()
                        .commit(None, "system", |w| store::drop_tentative(w.tx, &p, &key))?;
                }
                TicketStatus::Closed => {
                    closed = true;
                    if whole.is_none() {
                        whole = Some(self.map_whole(project)?);
                    }
                    let w = whole.as_ref().expect("read above");
                    let at = agenium_core::now_iso();
                    let links: Vec<&MapLink> = w
                        .links
                        .iter()
                        .filter(|l| l.status == TENTATIVE && l.ticket.as_deref() == Some(&key))
                        .collect();
                    // A confirmed link's ends are confirmed with it, whoever added them: a cancelled ticket then
                    // cannot take them away.
                    let ends: HashSet<&str> = links
                        .iter()
                        .flat_map(|l| [l.src.as_str(), l.dst.as_str()])
                        .collect();
                    self.store().commit(None, "system", |c| {
                        for n in w.nodes.values().filter(|n| {
                            n.status == TENTATIVE
                                && (n.ticket.as_deref() == Some(&key)
                                    || ends.contains(n.key.as_str()))
                        }) {
                            let mut n = n.clone();
                            n.status = CONFIRMED.into();
                            n.checked_at = at.clone();
                            store::put_node(c.tx, &p, &n)?;
                        }
                        for l in &links {
                            store::set_link_status(c.tx, &p, l.id, CONFIRMED, &l.evidence, &at)?;
                        }
                        Ok::<(), agenium_store::StoreError>(())
                    })?;
                }
                _ => {}
            }
        }
        if closed {
            self.map_recheck(project)?;
        }
        Ok(())
    }

    /// Checks every confirmed or stale piece of code evidence on the project's main branch again.
    pub fn map_recheck(&self, project: ProjectId) -> Result<()> {
        let proj = self.project(project)?;
        let root = &crate::project_create::checkout_dir(Path::new(&proj.path));
        let p = project.to_string();
        let w = self.map_whole(project)?;
        let at = agenium_core::now_iso();
        let (changes, nodes) = verify::cached(|| {
            let mut changes: Vec<(i64, &'static str, serde_json::Value)> = Vec::new();
            for l in w.links.iter().filter(|l| l.status != TENTATIVE) {
                let Ok(ev @ Evidence::Code { .. }) = Evidence::parse(&l.evidence) else {
                    continue;
                };
                let (Some(s), Some(d)) = (w.nodes.get(&l.src), w.nodes.get(&l.dst)) else {
                    continue;
                };
                let (Ok(src), Ok(dst)) = (
                    Ref::parse(&format!("{}:{}", s.kind, s.name)),
                    Ref::parse(&format!("{}:{}", d.kind, d.name)),
                ) else {
                    continue;
                };
                match verify::moved(root, &ev, &src, &dst) {
                    Some(now) if now != ev || l.status != CONFIRMED => {
                        changes.push((l.id, CONFIRMED, now.json()))
                    }
                    Some(_) => {}
                    None if l.status != STALE => changes.push((l.id, STALE, l.evidence.clone())),
                    None => {}
                }
            }
            let mut nodes: Vec<MapNode> = Vec::new();
            for n in w
                .nodes
                .values()
                .filter(|n| n.status != TENTATIVE && matches!(n.kind.as_str(), "file" | "symbol"))
            {
                let Ok(r) = Ref::parse(&format!("{}:{}", n.kind, n.name)) else {
                    continue;
                };
                let mut n = n.clone();
                match verify::locate(root, &r) {
                    Ok((a, b)) => {
                        if n.status != CONFIRMED || n.line_start != Some(a) || n.line_end != Some(b)
                        {
                            n.status = CONFIRMED.into();
                            n.line_start = Some(a);
                            n.line_end = Some(b);
                            n.checked_at = at.clone();
                            nodes.push(n);
                        }
                    }
                    Err(_) if n.status != STALE => {
                        n.status = STALE.into();
                        n.checked_at = at.clone();
                        nodes.push(n);
                    }
                    Err(_) => {}
                }
            }
            (changes, nodes)
        });
        if changes.is_empty() && nodes.is_empty() {
            return Ok(());
        }
        self.store().commit(None, "system", |c| {
            for (id, status, ev) in &changes {
                store::set_link_status(c.tx, &p, *id, status, ev, &at)?;
            }
            for n in &nodes {
                store::put_node(c.tx, &p, n)?;
            }
            Ok::<(), agenium_store::StoreError>(())
        })?;
        Ok(())
    }
}

/// One link as agents read it.
pub(crate) fn line_text(l: &MapLink) -> String {
    let status = match l.status.as_str() {
        CONFIRMED => String::new(),
        TENTATIVE => format!(
            " (tentative: {})",
            l.ticket.as_deref().unwrap_or("this run")
        ),
        s => format!(" ({s}: its evidence no longer matches; check it again)"),
    };
    format!(
        "{} --{}--> {}  [{}]{status}",
        l.src,
        l.kind,
        l.dst,
        evidence_text(&l.evidence)
    )
}
