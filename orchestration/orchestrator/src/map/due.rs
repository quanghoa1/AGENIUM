//! Whether a ticket's work run should put something on the project map before it reports (ADR-166 amended,
//! 2026-11-18). Measured: told only "map what you trace", agents mapped nothing, even a three-module code base, a
//! review of one, or a tool researched on the web. So at `submit_report` Agenium looks at what the run did, once:
//! - **a code base:** the worktree holds at least [`MIN_SOURCE_FILES`] source files, and the source files the ticket
//!   changed are not on the map (or, for work that changed none, such as a review, nothing of the code is);
//! - **a system researched:** the run read at least [`MIN_PAGES`] web pages.
//!
//! A run that already called `map_add` is not reminded; a one-file script or a change to code already mapped is not
//! either, so the map does not fill with noise. The agent may answer that the work is not worth mapping and report
//! again: it is a reminder, not a gate.

use std::collections::HashSet;
use std::path::Path;

use agenium_core::{ProjectId, RunId, TicketId};
use agenium_store::core::{runs, workspaces};

use crate::App;

/// Source files that make a code base worth mapping.
pub const MIN_SOURCE_FILES: usize = 3;
/// Web pages read that make a researched system worth mapping.
pub const MIN_PAGES: usize = 2;

const SOURCE: &[&str] = &[
    "py", "rs", "ts", "tsx", "js", "jsx", "mjs", "go", "java", "kt", "cs", "c", "h", "cc", "cpp",
    "hpp", "rb", "php", "swift", "scala", "lua", "sh", "ps1", "sql", "vue", "svelte", "dart", "ex",
    "exs", "mq4", "mq5", "mqh",
];
const SKIP: &[&str] = &[
    ".git",
    ".agenium",
    "node_modules",
    "target",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    "venv",
    ".next",
];

fn is_source(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, e)| SOURCE.contains(&e.to_ascii_lowercase().as_str()))
}

/// The source files under `root` (relative, forward slashes), at most 5 000 entries looked at.
fn source_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            seen += 1;
            if seen > 5_000 {
                return out;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if !SKIP.contains(&name.as_str()) {
                    stack.push(e.path());
                }
            } else if ft.is_file() && is_source(&name) {
                if let Ok(rel) = e.path().strip_prefix(root) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    out
}

/// Whether a tool line is a web read (a page fetched or opened, a search), the run's own tools or a CLI's.
fn web_read(tool: &str) -> bool {
    let t = tool.to_ascii_lowercase();
    [
        "fetch",
        "web_search",
        "browser",
        "webfetch",
        "websearch",
        "web_fetch",
    ]
    .iter()
    .any(|w| t.ends_with(w))
}

impl App {
    /// The reminder for run `run` of ticket `ticket` working in `root`, or `None` when nothing is due.
    pub fn map_due(
        &self,
        project: ProjectId,
        ticket: TicketId,
        run: RunId,
        root: &Path,
    ) -> Option<String> {
        let log = self.store().read(|c| runs::log_entries(c, run)).ok()?;
        let tools: Vec<(&str, bool)> = log
            .iter()
            .filter(|e| e.kind == "tool")
            .map(|e| {
                (
                    e.data["tool"].as_str().unwrap_or_default(),
                    e.data["ok"] != false,
                )
            })
            .collect();
        if tools.iter().any(|(t, ok)| *ok && t.ends_with("map_add")) {
            return None;
        }
        let pages = tools.iter().filter(|(t, ok)| *ok && web_read(t)).count();
        let sources = source_files(root);
        let code = sources.len() >= MIN_SOURCE_FILES && {
            let mapped: HashSet<String> = self
                .map_whole(project)
                .map(|w| w.nodes.values().filter_map(|n| n.file.clone()).collect())
                .unwrap_or_default();
            let changed = self.changed_sources(project, ticket, root);
            if changed.is_empty() {
                // Work that changed no code (a review): due while nothing of this code base is on the map.
                !sources.iter().any(|s| mapped.contains(s))
            } else {
                changed.iter().any(|c| !mapped.contains(c))
            }
        };
        let web = pages >= MIN_PAGES;
        if !code && !web {
            return None;
        }
        let mut asks = Vec::new();
        if code {
            asks.push(format!(
                "this work is in a code base of {} source files: put how the files you built, changed or read depend on \
                 each other on the map (file:<path> and symbol:<path>#<name> nodes; imports and calls as links, each \
                 with the file and line of the source that shows it, e.g. {{\"from\": \"file:a/api.py\", \"type\": \
                 \"imports\", \"to\": \"file:a/db.py\", \"evidence\": {{\"file\": \"a/api.py\", \"line\": 1}}}})",
                sources.len()
            ));
        }
        if web {
            asks.push(format!(
                "you read {pages} web pages: when they are about a system, a product or a vendor, put what you learned on \
                 the map (product:<name>, domain:<host>, service:<name>, endpoint:<what> nodes; links such as hosted_at, \
                 documented_at, releases_at, requests, each with the {{\"url\"}} of a page you read that names both \
                 ends, e.g. {{\"from\": \"product:ripgrep\", \"type\": \"hosted_at\", \"to\": \
                 \"domain:github.com\", \"evidence\": {{\"url\": \"https://github.com/BurntSushi/ripgrep\"}}}})"
            ));
        }
        Some(format!(
            "Before you report, the project map (map_add; it is what the next agent reads instead of tracing again): {}. \
             Agenium checks each entry and says why it refuses one; fix it and add again. Then call submit_report \
             again. Skip it only when there is nothing to connect (the pages were not about a system; the code is one \
             file), and say so in one line of your report. Asked once.",
            asks.join("; and ")
        ))
    }

    /// The source files ticket `ticket` changed in `root`: committed since its base and not committed yet.
    fn changed_sources(&self, project: ProjectId, ticket: TicketId, root: &Path) -> Vec<String> {
        let (Ok(Some(ws)), Ok(p)) = (
            self.store().read(|c| workspaces::get(c, ticket)),
            self.project(project),
        ) else {
            return vec![];
        };
        let repo = self.repo(&p);
        let mut out: Vec<String> = repo
            .changed_files(root, &ws.base_sha, "HEAD")
            .unwrap_or_default();
        // `XY path` lines (a rename: `old -> new`).
        out.extend(
            repo.status(root)
                .unwrap_or_default()
                .iter()
                .filter_map(|l| {
                    l.get(3..).map(|p| {
                        p.rsplit(" -> ")
                            .next()
                            .unwrap_or(p)
                            .trim_matches('"')
                            .to_owned()
                    })
                }),
        );
        out.retain(|f| is_source(f));
        out.sort();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_files_and_web_reads_are_told_apart() {
        assert!(is_source("shop/api.py") && is_source("src/Main.RS") && !is_source("README.md"));
        assert!(
            web_read("fetch") && web_read("cli:WebSearch") && web_read("mcp__agenium__browser")
        );
        assert!(!web_read("read_file") && !web_read("shell"));
        let d = tempfile::tempdir().unwrap();
        for f in ["a.py", "pkg/b.py", "node_modules/x.js", "notes.md"] {
            let p = d.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let mut s = source_files(d.path());
        s.sort();
        assert_eq!(s, ["a.py", "pkg/b.py"]);
    }
}
