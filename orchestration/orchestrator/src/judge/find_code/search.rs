//! The search: folders → files → units. A small tree skips the folders. Each step asks the judge one criterion over
//! many items (`rate`) and keeps what it rates at or above the step's bar; a step that keeps nothing falls back to its
//! best few, so a vague question still returns leads.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{json, Value};

use super::files::Repo;
use super::outline::units;
use super::rate::rate;
use super::render::{render, FileHit, Found, UnitHit};
use crate::app::App;

/// A tree with at most this many files is read file by file, without the folder step.
const DIRECT_FILES: usize = 60;
/// Folders asked about in one search, at most.
const MAX_DIRS: usize = 160;
/// Files asked about in one search, at most, and of them documents (Markdown, text) at most: code comes first — on
/// this repository, 130 ADRs crowded the code out of a 160-file budget.
const MAX_FILES: usize = 100;
const MAX_DOCS: usize = 25;
/// Files whose units are read.
const TOP_FILES: usize = 6;
/// Units asked about per file, at most: more in the two best files (a 1 600-line file holds the answer at line 939).
const UNITS_TOP: usize = 80;
const UNITS_PER_FILE: usize = 30;
/// Bars: a folder worth opening, a relevant file, a unit worth returning.
const DIR_BAR: f64 = 0.5;
const FILE_BAR: f64 = 0.5;
const UNIT_BAR: f64 = 0.6;
/// A file's head shown to the judge.
const HEAD_LINES: usize = 30;
const HEAD_CHARS: usize = 1_500;
/// A unit's source shown to the judge.
const UNIT_CHARS: usize = 6_000;

const DIR_CRITERION: &str = "Is this folder worth opening to answer the query? Use its path and the names of its \
files and subfolders as evidence; a partial listing is not proof that useful files are absent. Judge whether reading \
further is worthwhile, not whether everything inside is relevant.";

const FILE_CRITERION: &str = "Does this file hold concrete implementation, a caller, configuration or a test that \
would help a coding agent investigate the behaviour in the query? Judge from its path, its outline and the start of \
its source. Code that controls or carries the behaviour counts even when it is not where the change will be made; \
shared words, unrelated utilities and incidental imports do not. Plans, notes and specs count only when the query \
asks about design or documentation.";

const UNIT_CRITERION: &str = "Does this exact unit of source implement, control or directly test the behaviour in \
the query? Count the current implementation even if it is wrong or incomplete: this selects code to investigate. Judge \
this unit itself, not its whole file. Similar topic words, generic helpers and narrative text are not enough.";

/// Indices of `p` at or above `bar`, best first; when none is, the best `fallback` answered ones.
fn kept(p: &[Option<f64>], bar: f64, fallback: usize) -> Vec<usize> {
    let mut ranked: Vec<(usize, f64)> = p
        .iter()
        .enumerate()
        .filter_map(|(i, x)| x.map(|x| (i, x)))
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let above: Vec<usize> = ranked
        .iter()
        .filter(|(_, x)| *x >= bar)
        .map(|(i, _)| *i)
        .collect();
    if above.is_empty() {
        ranked.iter().take(fallback).map(|(i, _)| *i).collect()
    } else {
        above
    }
}

fn ext(path: &str) -> &str {
    path.rsplit_once('.').map_or("", |(_, e)| e)
}

/// Runs the search under `root` (shown as `shown_root`) and renders what it found.
pub(super) async fn search(
    app: &App,
    root: &Path,
    shown_root: &str,
    question: &str,
    ticket: Option<&str>,
) -> String {
    let started = std::time::Instant::now();
    let repo = {
        let root = root.to_path_buf();
        tokio::task::spawn_blocking(move || Repo::scan(&root))
            .await
            .expect("scan")
    };
    let mut notes = Vec::new();
    if repo.truncated {
        notes.push(
            "the tree has more files than one search lists: search a narrower folder".to_owned(),
        );
    }
    let (mut tokens, mut requests, mut failed) = (0, 0, 0);

    // Folders → candidate files, each with the probability of the folder it came from.
    let mut candidates: Vec<(String, f64)> = Vec::new();
    if repo.files.len() <= DIRECT_FILES {
        candidates = repo.files.iter().map(|f| (f.clone(), 1.0)).collect();
    } else {
        let tree = repo.tree();
        let mut frontier: Vec<(String, f64)> = vec![(String::new(), 1.0)];
        let mut asked = 0;
        while !frontier.is_empty() {
            let mut subdirs: Vec<(String, f64)> = Vec::new();
            for (d, p) in &frontier {
                let (kids, files) = &tree[d];
                candidates.extend(files.iter().map(|f| (f.clone(), *p)));
                subdirs.extend(kids.iter().map(|k| (k.clone(), *p)));
            }
            if subdirs.is_empty() {
                break;
            }
            if asked + subdirs.len() > MAX_DIRS {
                notes.push("not every folder was opened (the folder budget ran out)".to_owned());
                subdirs.truncate(MAX_DIRS - asked);
            }
            if subdirs.is_empty() {
                break;
            }
            asked += subdirs.len();
            let items: Vec<Value> = subdirs
                .iter()
                .map(|(d, _)| {
                    let (kids, files) = &tree[d];
                    let name = |p: &String| p.rsplit('/').next().unwrap_or(p).to_owned();
                    json!({
                        "path": format!("{d}/"),
                        "subfolders": kids.iter().take(30).map(name).collect::<Vec<_>>(),
                        "files": files.iter().take(40).map(name).collect::<Vec<_>>(),
                        "files_below": repo.files.iter().filter(|f| f.starts_with(&format!("{d}/"))).count(),
                    })
                })
                .collect();
            let r = rate(app, question, DIR_CRITERION, &items, ticket).await;
            (tokens, requests, failed) = (
                tokens + r.input_tokens,
                requests + r.requests,
                failed + r.failed,
            );
            frontier = kept(&r.p, DIR_BAR, 2)
                .into_iter()
                .map(|i| (subdirs[i].0.clone(), r.p[i].unwrap_or(0.0)))
                .collect();
        }
    }
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let doc = |f: &str| matches!(ext(f), "md" | "markdown" | "txt" | "rst" | "adoc");
    let docs = candidates.iter().filter(|(f, _)| doc(f)).count();
    if docs > MAX_DOCS {
        let mut kept_docs = 0;
        candidates.retain(|(f, _)| {
            if !doc(f) {
                return true;
            }
            kept_docs += 1;
            kept_docs <= MAX_DOCS
        });
        notes.push(format!(
            "{} documents were not read (code comes first)",
            docs - MAX_DOCS
        ));
    }
    // Code before documents within the budget.
    candidates.sort_by_key(|(f, _)| doc(f));
    if candidates.len() > MAX_FILES {
        notes.push(format!(
            "{} files in the opened folders were not read (the file budget ran out)",
            candidates.len() - MAX_FILES
        ));
        candidates.truncate(MAX_FILES);
    }

    // Files: path, outline and head.
    let mut texts: HashMap<String, String> = HashMap::new();
    let mut paths = Vec::new();
    let mut items = Vec::new();
    for (f, _) in &candidates {
        let Some(t) = repo.text(f) else { continue };
        let outline: Vec<String> = units(ext(f), &t)
            .iter()
            .take(40)
            .map(|u| format!("{} ({}-{})", u.name, u.start, u.end))
            .collect();
        let head: String = t
            .lines()
            .take(HEAD_LINES)
            .collect::<Vec<_>>()
            .join("\n")
            .chars()
            .take(HEAD_CHARS)
            .collect();
        items.push(json!({ "path": f, "outline": outline, "head": head }));
        paths.push(f.clone());
        texts.insert(f.clone(), t);
    }
    let r = rate(app, question, FILE_CRITERION, &items, ticket).await;
    (tokens, requests, failed) = (
        tokens + r.input_tokens,
        requests + r.requests,
        failed + r.failed,
    );
    let files: Vec<FileHit> = kept(&r.p, FILE_BAR, 3)
        .into_iter()
        .map(|i| FileHit {
            path: paths[i].clone(),
            p: r.p[i].unwrap_or(0.0),
        })
        .collect();

    // Units of the best files.
    let mut unit_items = Vec::new();
    let mut unit_meta = Vec::new();
    for (rank, h) in files.iter().take(TOP_FILES).enumerate() {
        let t = &texts[&h.path];
        let lines: Vec<&str> = t.lines().collect();
        for u in units(ext(&h.path), t).into_iter().take(if rank < 2 {
            UNITS_TOP
        } else {
            UNITS_PER_FILE
        }) {
            let source = lines[u.start - 1..u.end.min(lines.len())].join("\n");
            unit_items.push(json!({
                "path": h.path,
                "unit": u.name,
                "lines": format!("{}-{}", u.start, u.end),
                "source": source.chars().take(UNIT_CHARS).collect::<String>(),
            }));
            unit_meta.push((h.path.clone(), u, source));
        }
    }
    let r = rate(app, question, UNIT_CRITERION, &unit_items, ticket).await;
    (tokens, requests, failed) = (
        tokens + r.input_tokens,
        requests + r.requests,
        failed + r.failed,
    );
    let rank: HashMap<&str, usize> = files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.path.as_str(), i))
        .collect();
    let mut chosen: Vec<UnitHit> = unit_meta
        .iter()
        .zip(&r.p)
        .filter(|(_, p)| p.is_some_and(|p| p >= UNIT_BAR))
        .map(|((path, u, source), p)| UnitHit {
            path: path.clone(),
            name: u.name.clone(),
            start: u.start,
            end: u.end,
            p: p.unwrap_or(0.0),
            source: source.clone(),
        })
        .collect();
    chosen.sort_by(|a, b| {
        rank[a.path.as_str()]
            .cmp(&rank[b.path.as_str()])
            .then(b.p.total_cmp(&a.p))
            .then(a.start.cmp(&b.start))
    });
    if failed > 0 {
        notes.push(format!(
            "{failed} of {requests} judge requests failed: their items are unknown, not irrelevant"
        ));
    }

    let asked_files = paths.len();
    app.record(
        "find_code",
        ticket,
        format!(
            "{:?}: {} relevant file(s), {} excerpt(s) from {} files read ({requests} requests)",
            question.chars().take(80).collect::<String>(),
            files.len(),
            chosen.len(),
            asked_files
        ),
        &crate::judge::client::Reply {
            answers: Default::default(),
            ms: started.elapsed().as_millis() as u64,
            usage: Some(json!({ "input_tokens": tokens })),
            cached: false,
            key: None,
        },
    );
    render(&Found {
        question,
        root: shown_root,
        files,
        units: chosen,
        searched_files: repo.files.len(),
        asked_files,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_takes_what_passes_the_bar_or_the_best_few() {
        let p = [Some(0.2), Some(0.9), None, Some(0.6), Some(0.4)];
        assert_eq!(kept(&p, 0.5, 2), [1, 3]);
        assert_eq!(kept(&p, 0.95, 2), [1, 3], "nothing passes: the best two");
        assert!(kept(&[None, None], 0.5, 2).is_empty());
    }
}
