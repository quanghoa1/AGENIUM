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

/// The literal part of a glob before its first metacharacter, with `\` normalised to `/`.
fn literal_prefix(glob: &str) -> (String, bool) {
    let g = glob.replace('\\', "/");
    let g = g.trim_start_matches("./");
    match g.find(['*', '?', '[', '{']) {
        Some(i) => (g[..i].to_owned(), true),
        None => (g.to_owned(), false),
    }
}

/// Conservative test whether two scope glob lists can touch the same file. Over-approximates (two globs whose
/// literal directory prefixes nest are treated as overlapping); never misses a real overlap of literal paths or
/// of a literal path inside a `dir/**` glob.
pub fn scopes_overlap(a: &[String], b: &[String]) -> bool {
    a.iter().any(|x| b.iter().any(|y| glob_pair_overlaps(x, y)))
}

fn glob_pair_overlaps(x: &str, y: &str) -> bool {
    let (px, gx) = literal_prefix(x);
    let (py, gy) = literal_prefix(y);
    match (gx, gy) {
        (false, false) => px == py,
        (true, false) => py.starts_with(&px),
        (false, true) => px.starts_with(&py),
        (true, true) => px.starts_with(&py) || py.starts_with(&px),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
