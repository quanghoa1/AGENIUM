//! The skills a folder holds (a collection: one skill per subfolder, or several levels of them), for importing many at
//! once. Each is checked as an import would check it; nothing is copied here.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{validate_named, Origin, SKILL_FILE};

/// How deep under the chosen folder skills are looked for (`collection/category/skill/SKILL.md` is 3).
const DEPTH: usize = 4;
/// At most this many are listed: a folder that holds more is not a skill collection.
const MAX: usize = 300;

/// A skill folder found by [`scan`].
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Found {
    pub path: PathBuf,
    /// From its frontmatter; the folder's name when it cannot be read.
    pub name: String,
    pub description: String,
    /// Why it cannot be imported as it is (a broken frontmatter, a missing reference, too big).
    pub error: Option<String>,
}

/// Every skill folder under `dir` (itself included), in path order. A skill's own subfolders are not searched (its
/// references); hidden folders, `node_modules` and `target` are skipped.
pub fn scan(dir: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    walk(dir, 0, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<Found>) {
    if out.len() >= MAX {
        return;
    }
    if dir.join(SKILL_FILE).is_file() {
        let fallback = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        out.push(match validate_named(dir, Origin::Agent, false) {
            Ok(s) => Found {
                path: dir.to_path_buf(),
                name: s.meta.name,
                description: s.meta.description,
                error: None,
            },
            Err(e) => Found {
                path: dir.to_path_buf(),
                name: fallback,
                description: String::new(),
                error: Some(e.to_string()),
            },
        });
        return;
    }
    if depth >= DEPTH {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut dirs: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            !n.starts_with('.') && n != "node_modules" && n != "target"
        })
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for d in dirs {
        walk(&d, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join(SKILL_FILE),
            format!("---\nname: {name}\ndescription: Use when testing {name}.\n---\nSteps.\n"),
        )
        .unwrap();
    }

    #[test]
    fn a_collection_lists_each_skill_once_with_what_is_wrong() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        skill(&root.join("pdf"), "pdf");
        skill(&root.join("design").join("brand-colors"), "brand-colors");
        // A skill's own subfolder is not a second skill.
        skill(&root.join("pdf").join("examples").join("inner"), "inner");
        std::fs::create_dir_all(root.join("broken")).unwrap();
        std::fs::write(root.join("broken").join(SKILL_FILE), "no frontmatter\n").unwrap();
        skill(&root.join(".git").join("hidden"), "hidden");
        skill(&root.join("node_modules").join("dep"), "dep");

        let found = scan(root);
        let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["broken", "brand-colors", "pdf"]);
        assert!(found[0].error.is_some());
        assert_eq!(found[2].description, "Use when testing pdf.");
        assert!(found[1].error.is_none() && found[2].error.is_none());

        // A skill folder chosen directly is that one skill.
        let one = scan(&root.join("pdf"));
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].name, "pdf");
    }
}
