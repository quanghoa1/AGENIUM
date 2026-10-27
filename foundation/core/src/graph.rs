//! Ticket graph helpers: the `depends_on` DAG check and scope overlap (decision 18 prevention rule).

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

/// If adding the edges `node -> deps` to the dependency graph `edges` (ticket -> its dependencies) would create a
/// cycle, returns one offending path `node -> ... -> node`.
pub fn find_cycle<K: Eq + Hash + Clone>(
    edges: &HashMap<K, Vec<K>>,
    node: &K,
    deps: &[K],
) -> Option<Vec<K>> {
    // A cycle exists iff `node` is reachable from one of its new dependencies.
    for d in deps {
        if d == node {
            return Some(vec![node.clone(), node.clone()]);
        }
        let mut stack: Vec<(K, Vec<K>)> = vec![(d.clone(), vec![node.clone(), d.clone()])];
        let mut seen: HashSet<K> = HashSet::new();
        while let Some((cur, path)) = stack.pop() {
            if !seen.insert(cur.clone()) {
                continue;
            }
            for next in edges.get(&cur).map(Vec::as_slice).unwrap_or_default() {
                let mut p = path.clone();
                p.push(next.clone());
                if next == node {
                    return Some(p);
                }
                stack.push((next.clone(), p));
            }
        }
    }
    None
}

/// The literal part of a glob before its first metacharacter, with `\` normalised to `/` (a pattern without
/// metacharacters loses its trailing `/`: `src/` is the folder `src`).
fn literal_prefix(glob: &str) -> (String, bool) {
    let g = glob.trim().replace('\\', "/");
    let g = g.trim_start_matches("./");
    match g.find(['*', '?', '[', '{']) {
        Some(i) => (g[..i].to_owned(), true),
        None => (g.trim_end_matches('/').to_owned(), false),
    }
}

/// `path` is the literal `dir` or lies below it.
fn under(path: &str, dir: &str) -> bool {
    path.strip_prefix(dir)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Conservative test whether two scope glob lists can touch the same file. Over-approximates (two globs whose
/// literal directory prefixes nest are treated as overlapping); never misses a real overlap. A literal path is read
/// as the write scope reads it (agenium-tools `WriteScope`): `p` covers `p` and everything below it, so `src` and
/// `src/main.rs` overlap.
pub fn scopes_overlap(a: &[String], b: &[String]) -> bool {
    a.iter().any(|x| b.iter().any(|y| glob_pair_overlaps(x, y)))
}

/// The files a ticket names in `text` (its title and acceptance criteria), for a ticket that declares no scope
/// (ADR-120): a path with a folder and a file extension (`deliverables/check.py`), or a file name in backticks
/// (`` `check.py` ``). Folders alone, URLs and names such as `Node.js` outside backticks are not taken. Lower case.
pub fn paths_named(text: &str) -> Vec<String> {
    let file_like = |p: &str| {
        let last = p.rsplit('/').next().unwrap_or(p);
        let ext = last.rsplit_once('.');
        matches!(ext, Some((n, e)) if !n.is_empty() && (1..=6).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric()) && e.chars().any(|c| c.is_ascii_alphabetic()))
    };
    let clean = |t: &str| -> String {
        t.trim_matches(|c: char| !(c.is_ascii_alphanumeric() || "_./-".contains(c)))
            .trim_end_matches('.')
            .trim_start_matches("./")
            .replace('\\', "/")
            .to_lowercase()
    };
    let mut out: Vec<String> = Vec::new();
    let mut push = |p: String| {
        if !p.is_empty() && !out.contains(&p) {
            out.push(p);
        }
    };
    for (i, part) in text.split('`').enumerate() {
        let quoted = i % 2 == 1;
        for tok in part.split(|c: char| c.is_whitespace() || ",;()[]{}\"'<>".contains(c)) {
            if tok.contains("://") || tok.starts_with("//") {
                continue;
            }
            let p = clean(tok);
            if file_like(&p)
                && (p.contains('/') || quoted)
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c))
            {
                push(p);
            }
        }
    }
    out
}

fn glob_pair_overlaps(x: &str, y: &str) -> bool {
    let (px, gx) = literal_prefix(x);
    let (py, gy) = literal_prefix(y);
    match (gx, gy) {
        (false, false) => under(&px, &py) || under(&py, &px),
        (true, false) => py.starts_with(&px) || under(&px, &py),
        (false, true) => px.starts_with(&py) || under(&py, &px),
        (true, true) => px.starts_with(&py) || py.starts_with(&px),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_named_by_a_ticket() {
        assert_eq!(
            paths_named("Create deliverables/hello.txt and deliverables/check.py, run the script."),
            ["deliverables/hello.txt", "deliverables/check.py"]
        );
        assert_eq!(
            paths_named("Update the `README.md` usage section"),
            ["readme.md"]
        );
        for none in [
            "Write files under deliverables/ only",
            "Built with Node.js and served at https://example.com/app.js",
            "Bump version to 1.2.3",
        ] {
            assert!(
                paths_named(none).is_empty(),
                "{none}: {:?}",
                paths_named(none)
            );
        }
        assert_eq!(paths_named(r"Fix src\main.rs"), ["src/main.rs"]);
    }

    #[test]
    fn detects_cycles() {
        let mut g: HashMap<&str, Vec<&str>> = HashMap::new();
        g.insert("B", vec!["C"]);
        g.insert("C", vec!["A"]);
        assert_eq!(find_cycle(&g, &"A", &["B"]), Some(vec!["A", "B", "C", "A"]));
        assert_eq!(find_cycle(&g, &"A", &["A"]), Some(vec!["A", "A"]));
        assert_eq!(find_cycle(&g, &"D", &["B"]), None);
        assert_eq!(find_cycle(&g, &"A", &[]), None);
    }

    #[test]
    fn scope_overlap_is_conservative() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(scopes_overlap(&s(&["src/auth/**"]), &s(&["src/**"])));
        assert!(scopes_overlap(
            &s(&["src/auth/**"]),
            &s(&["src/auth/login.rs"])
        ));
        assert!(scopes_overlap(&s(&["src/a.rs"]), &s(&["./src/a.rs"])));
        assert!(!scopes_overlap(&s(&["src/a.rs"]), &s(&["src/b.rs"])));
        assert!(!scopes_overlap(&s(&["src/auth/**"]), &s(&["docs/**"])));
        assert!(!scopes_overlap(&s(&[]), &s(&["**"])));
        assert!(scopes_overlap(&s(&["**"]), &s(&["docs/x.md"])));
    }

    /// A literal scope is a folder too (the write scope lets `src` write `src/main.rs`): two tickets scoped `src` and
    /// `src/main.rs` can touch the same file. A sibling with the same leading letters is not inside it.
    #[test]
    fn a_literal_scope_covers_what_is_below_it() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(scopes_overlap(&s(&["src"]), &s(&["src/main.rs"])));
        assert!(scopes_overlap(&s(&["src/main.rs"]), &s(&["src/"])));
        assert!(scopes_overlap(&s(&["src\\"]), &s(&["./src/lib.rs"])));
        assert!(scopes_overlap(&s(&["src"]), &s(&["src/auth/**"])));
        assert!(scopes_overlap(&s(&["src/auth/**"]), &s(&["src"])));
        assert!(!scopes_overlap(&s(&["src"]), &s(&["src2/main.rs"])));
        assert!(!scopes_overlap(&s(&["src"]), &s(&["srcx/**"])));
        assert!(!scopes_overlap(&s(&["src/a"]), &s(&["src/ab.rs"])));
    }
}
