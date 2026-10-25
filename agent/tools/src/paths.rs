//! Path confinement (decision 16): every path is normalised and must stay inside the ticket worktree (reads may
//! also use the read-only roots, e.g. skill folders). Symlinks are resolved on the existing part of the path so a
//! link cannot escape. `.git` is never writable.

use std::path::{Component, Path, PathBuf};

use crate::ToolError;

fn lexical_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn key(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s
        .trim_start_matches("//?/")
        .trim_end_matches('/')
        .to_owned();
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

/// `true` when `p` is `root` or below it (component-wise, case-insensitive on Windows).
pub fn within(p: &Path, root: &Path) -> bool {
    let (p, r) = (key(p), key(root));
    p == r || p.starts_with(&format!("{r}/"))
}

/// Canonical form of the deepest existing ancestor of `p`, with the remaining components re-appended.
fn resolve_existing(p: &Path) -> PathBuf {
    let mut existing = p.to_path_buf();
    let mut rest = Vec::new();
    while !existing.exists() {
        match (
            existing.file_name().map(|n| n.to_owned()),
            existing.parent().map(Path::to_path_buf),
        ) {
            (Some(n), Some(parent)) => {
                rest.push(n);
                existing = parent;
            }
            _ => break,
        }
    }
    let mut c = std::fs::canonicalize(&existing).unwrap_or(existing);
    for n in rest.into_iter().rev() {
        c.push(n);
    }
    c
}

/// Resolves a tool path. `write` additionally forbids `.git` and requires the worktree root.
pub fn resolve(
    root: &Path,
    read_roots: &[PathBuf],
    raw: &str,
    write: bool,
) -> Result<PathBuf, ToolError> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains('\0') {
        return Err(ToolError::new("invalid_path", "path is empty"));
    }
    let p = Path::new(raw);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    };
    let norm = lexical_normalize(&joined);
    let real = resolve_existing(&norm);
    // The worktree has two names when a symlink or a short name leads to it (macOS: /var -> /private/var; Windows:
    // QUANGH~1 -> the long name), and a CLI that asks the OS for its working directory writes the canonical one:
    // both name the worktree. The resolved path must still be inside it, so a link cannot escape.
    let canon_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let in_root = (within(&norm, root) || within(&norm, &canon_root)) && within(&real, &canon_root);
    let in_read_root = !write
        && read_roots.iter().any(|r| {
            let cr = std::fs::canonicalize(r).unwrap_or_else(|_| r.clone());
            (within(&norm, r) || within(&norm, &cr)) && within(&real, &cr)
        });
    if !(in_root || in_read_root) {
        return Err(ToolError::new(
            "out_of_scope",
            format!("{raw} is outside your worktree; to touch other paths use request_permission (kind=path)"),
        ));
    }
    if write
        && norm
            .components()
            .any(|c| c.as_os_str().to_string_lossy().eq_ignore_ascii_case(".git"))
    {
        return Err(ToolError::new(
            "read_only",
            ".git is read-only; the orchestrator manages commits and refs",
        ));
    }
    Ok(norm)
}

/// Where an agent may write inside its worktree (team mode, architecture page "Ép phạm vi"): every list in
/// `all_of` must match (an empty list matches everything), e.g. the ticket's `scope_paths` and the role's allowed
/// paths; approved path permissions match on their own. Patterns are worktree-relative globs; a pattern without
/// glob characters also covers everything below it.
#[derive(Debug, Clone, Default)]
pub struct WriteScope {
    all_of: Vec<(Vec<String>, globset::GlobSet)>,
    granted: (Vec<String>, globset::GlobSet),
}

fn glob_set(patterns: &[String]) -> Result<globset::GlobSet, String> {
    let mut b = globset::GlobSetBuilder::new();
    for p in patterns {
        let p = p.trim().trim_start_matches("./").trim_end_matches('/');
        let mut add = |g: &str| -> Result<(), String> {
            b.add(
                globset::GlobBuilder::new(g)
                    .literal_separator(true)
                    .build()
                    .map_err(|e| format!("scope pattern {g:?}: {e}"))?,
            );
            Ok(())
        };
        add(p)?;
        if !p.contains(['*', '?', '[', '{']) {
            add(&format!("{p}/**"))?;
        }
    }
    b.build().map_err(|e| e.to_string())
}

impl WriteScope {
    pub fn new(all_of: Vec<Vec<String>>, granted: Vec<String>) -> Result<Self, String> {
        let all_of = all_of
            .into_iter()
            .filter(|l| !l.is_empty())
            .map(|l| glob_set(&l).map(|g| (l, g)))
            .collect::<Result<_, _>>()?;
        let g = glob_set(&granted)?;
        Ok(Self {
            all_of,
            granted: (granted, g),
        })
    }

    /// No restriction (solo work, tickets without scope).
    pub fn is_open(&self) -> bool {
        self.all_of.is_empty()
    }

    /// `rel`: worktree-relative path with forward slashes.
    pub fn allows(&self, rel: &str) -> bool {
        self.is_open()
            || self.granted.1.is_match(rel)
            || self.all_of.iter().all(|(_, g)| g.is_match(rel))
    }

    pub fn describe(&self) -> String {
        let mut s = self
            .all_of
            .iter()
            .map(|(l, _)| l.join(", "))
            .collect::<Vec<_>>()
            .join("] and [");
        s = format!("[{s}]");
        if !self.granted.0.is_empty() {
            s.push_str(&format!(" or granted [{}]", self.granted.0.join(", ")));
        }
        s
    }
}

/// Path relative to the worktree for display (forward slashes).
pub fn display(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_scope() {
        let open = WriteScope::default();
        assert!(open.is_open() && open.allows("anything.rs"));
        let s = WriteScope::new(
            vec![
                vec!["src/auth/**".into(), "tests".into()],
                vec!["src/**".into(), "tests/**".into()],
                vec![],
            ],
            vec!["docs/AUTH.md".into()],
        )
        .unwrap();
        assert!(s.allows("src/auth/login.rs"));
        assert!(
            s.allows("tests/auth.rs"),
            "a plain directory covers its files"
        );
        assert!(!s.allows("src/billing/x.rs"), "outside the ticket scope");
        assert!(s.allows("docs/AUTH.md"), "granted");
        assert!(!s.allows("docs/OTHER.md"));
        let role_only =
            WriteScope::new(vec![vec![], vec!["deliverables/web/**".into()]], vec![]).unwrap();
        assert!(
            role_only.allows("deliverables/web/f.md")
                && !role_only.allows("deliverables/mobile/f.md")
        );
        assert!(WriteScope::new(vec![vec!["a/[".into()]], vec![]).is_err());
    }

    #[test]
    fn confinement() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let skills = tmp.path().join("skills");
        std::fs::create_dir_all(&skills).unwrap();
        assert!(resolve(&root, &[], "src/a.rs", true).is_ok());
        assert!(resolve(&root, &[], "new/dir/file.txt", true).is_ok());
        assert!(resolve(&root, &[], "./src/../src/a.rs", true).is_ok());
        assert_eq!(
            resolve(&root, &[], "../outside.txt", true)
                .unwrap_err()
                .code,
            "out_of_scope"
        );
        assert_eq!(
            resolve(&root, &[], "src/../../x", false).unwrap_err().code,
            "out_of_scope"
        );
        let abs_out = tmp.path().join("other.txt");
        assert_eq!(
            resolve(&root, &[], abs_out.to_str().unwrap(), false)
                .unwrap_err()
                .code,
            "out_of_scope"
        );
        assert_eq!(
            resolve(&root, &[], ".git/config", true).unwrap_err().code,
            "read_only"
        );
        assert_eq!(
            resolve(&root, &[], "sub/.GIT/hooks/x", true)
                .unwrap_err()
                .code,
            "read_only"
        );
        assert!(
            resolve(&root, &[], ".git/config", false).is_ok(),
            "reading .git is fine"
        );
        let sk = skills.join("SKILL.md");
        assert!(resolve(
            &root,
            std::slice::from_ref(&skills),
            sk.to_str().unwrap(),
            false
        )
        .is_ok());
        assert!(
            resolve(&root, &[skills], sk.to_str().unwrap(), true).is_err(),
            "read roots are read-only"
        );
        assert!(resolve(&root, &[], "", false).is_err());
    }

    /// macOS run of 2026-10-25: the worktree was given as /var/folders/…/wt, the agent CLI wrote to its getcwd(),
    /// /private/var/folders/…/wt/a.txt, and the write was refused as outside the worktree. On Windows the temp dir
    /// is often a short name (`C:\Users\QUANGH~1\…`) whose canonical form is the long one.
    #[test]
    fn the_canonical_name_of_the_worktree_is_the_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt");
        std::fs::create_dir_all(&root).unwrap();
        let canon = std::fs::canonicalize(&root).unwrap();
        let inside = canon.join("a.txt");
        assert!(resolve(&root, &[], inside.to_str().unwrap(), true).is_ok());
        let outside = canon.join("..").join("escape.txt");
        assert_eq!(
            resolve(&root, &[], outside.to_str().unwrap(), true)
                .unwrap_err()
                .code,
            "out_of_scope"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_blocked() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt");
        std::fs::create_dir_all(&root).unwrap();
        std::os::unix::fs::symlink(tmp.path(), root.join("link")).unwrap();
        assert_eq!(
            resolve(&root, &[], "link/secret", false).unwrap_err().code,
            "out_of_scope"
        );
    }
}
