//! Path confinement (decision 16): every path is normalised and must stay inside the ticket worktree (reads may
//! also use the read-only roots, e.g. skill folders). Symlinks are resolved on the existing part of the path so a
//! link cannot escape; a dangling link resolves nowhere. `.git` is never writable, under any name that reaches it.

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
    // ASCII only: NTFS treats a Unicode case pair such as K and the Kelvin sign as two names, which a full lowercase
    // made one, letting a path in a twin folder beside the root pass as inside it.
    if cfg!(windows) {
        s.to_ascii_lowercase()
    } else {
        s
    }
}

/// `true` when `p` is `root` or below it (component-wise, case-insensitive on Windows).
pub fn within(p: &Path, root: &Path) -> bool {
    let (p, r) = (key(p), key(root));
    p == r || p.starts_with(&format!("{r}/"))
}

/// Canonical form of the deepest existing ancestor of `p`, with the remaining components re-appended. An entry exists
/// when it is there itself (`symlink_metadata`, which does not follow links): a dangling link is not a new name inside
/// the root, a write through it creates its target wherever that is (Linux/macOS). It resolves nowhere (`None`), as
/// does a link loop.
fn resolve_existing(p: &Path) -> Option<PathBuf> {
    let mut existing = p.to_path_buf();
    let mut rest = Vec::new();
    while std::fs::symlink_metadata(&existing).is_err() {
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
    let mut c = match std::fs::canonicalize(&existing) {
        Ok(c) => c,
        Err(_)
            if std::fs::symlink_metadata(&existing).is_ok_and(|m| m.file_type().is_symlink()) =>
        {
            return None
        }
        Err(_) => existing,
    };
    for n in rest.into_iter().rev() {
        c.push(n);
    }
    Some(c)
}

/// `true` when the absolute, lexically normalised `p` is inside `root`. The root has two names when a symlink or a
/// short name leads to it (macOS: /var -> /private/var; Windows: QUANGH~1 -> the long name), and a CLI that asks the
/// OS for its working directory writes the canonical one: both name the root. The resolved path must still be inside
/// it, so a link cannot escape. A root that does not exist yet is resolved like the path, through its deepest
/// existing ancestor.
pub fn confined(p: &Path, root: &Path) -> bool {
    let canon_root = resolve_existing(root).unwrap_or_else(|| root.to_path_buf());
    (within(p, root) || within(p, &canon_root))
        && resolve_existing(p).is_some_and(|r| within(&r, &canon_root))
}

/// A component that names `.git`, case-insensitively, also as Windows reads it: trailing dots and spaces dropped,
/// an alternate data stream (`.git::$INDEX_ALLOCATION`) is the folder itself.
fn is_git(name: &std::ffi::OsStr) -> bool {
    let s = name.to_string_lossy();
    let s = if cfg!(windows) {
        s.split(':')
            .next()
            .unwrap_or_default()
            .trim_end_matches(['.', ' '])
    } else {
        &s
    };
    s.eq_ignore_ascii_case(".git")
}

/// A name Windows takes for another one: trailing dots or spaces (dropped), a `:` (an alternate data stream of the
/// file before it), an 8.3 short name (`GIT~1`). Lexically it is not the name it reaches, so no rule on names can
/// hold for it.
fn windows_alias(name: &str) -> bool {
    let short = name
        .char_indices()
        .any(|(i, c)| c == '~' && name[i + 1..].starts_with(|d: char| d.is_ascii_digit()));
    name.ends_with(['.', ' ']) || name.contains(':') || short
}

/// Why a write to the absolute, lexically normalised `p` inside `root` is refused, if it is: `.git` is read-only (the
/// orchestrator owns commits, refs, hooks and config; a hook or `core.fsmonitor` written there runs on the host at
/// the next git call), under every name that reaches it — the path's own components, and the canonical form of its
/// existing part (a short name, a link). On Windows a name below the root that is an alias of another one is refused
/// outright (the root's own names are the user's: a temp folder is often `QUANGH~1`). The same rule holds for the
/// file tools of vendor CLIs (agenium-cli-backend).
pub fn write_refusal(root: &Path, p: &Path) -> Option<ToolError> {
    let names_git = |p: &Path| {
        p.components()
            .any(|c| matches!(c, Component::Normal(n) if is_git(n)))
    };
    if names_git(p) || resolve_existing(p).is_some_and(|r| names_git(&r)) {
        return Some(ToolError::new(
            "read_only",
            ".git is read-only; the orchestrator manages commits and refs",
        ));
    }
    let below = |r: &Path| {
        key(p)
            .strip_prefix(&format!("{}/", key(r)))
            .map(str::to_owned)
    };
    let rel = below(root)
        .or_else(|| resolve_existing(root).and_then(|c| below(&c)))
        .unwrap_or_default();
    if cfg!(windows) && rel.split('/').any(windows_alias) {
        return Some(ToolError::new(
            "invalid_path",
            format!(
                "{} has a name Windows reads as another one (trailing dot or space, ':' or an 8.3 short name like \
                 NAME~1); use the plain name",
                p.display()
            ),
        ));
    }
    None
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
    let in_root = confined(&norm, root);
    let in_read_root = !write && read_roots.iter().any(|r| confined(&norm, r));
    if !(in_root || in_read_root) {
        return Err(ToolError::new(
            "out_of_scope",
            format!("{raw} is outside your worktree; to touch other paths use request_permission (kind=path)"),
        ));
    }
    if let Some(e) = write.then(|| write_refusal(root, &norm)).flatten() {
        return Err(e);
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

/// Where `p` really leads, relative to the worktree (forward slashes, its case kept): the canonical form of its deepest
/// existing ancestor against the canonical worktree; `None` outside it. A write scope is checked on it too: a link
/// `src/auth/l -> ../billing` took a write to `src/auth/l/x.rs` into `src/billing` (path review of 2026-10-29).
pub fn real_display(root: &Path, p: &Path) -> Option<String> {
    let canon_root = resolve_existing(root)?;
    let real = resolve_existing(p)?;
    if !within(&real, &canon_root) {
        return None;
    }
    let rel: PathBuf = real
        .components()
        .skip(canon_root.components().count())
        .collect();
    Some(rel.to_string_lossy().replace('\\', "/"))
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

    /// macOS run 3: a worktree that did not exist yet, under /var (a link to /private/var), refused `src/a.rs`. The
    /// path resolved through /private/var, the root kept its /var name.
    #[cfg(unix)]
    #[test]
    fn a_root_not_created_yet_under_a_link_holds_its_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("real")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("real"), tmp.path().join("link")).unwrap();
        let root = tmp.path().join("link").join("wt");
        assert!(confined(&root.join("src").join("a.rs"), &root));
        assert!(!confined(&tmp.path().join("real").join("x.rs"), &root));
    }

    /// A link, or `false` when this machine may not make one (Windows without the symlink privilege).
    fn link(target: &Path, link: &Path, dir: bool) -> bool {
        #[cfg(unix)]
        let made = {
            let _ = dir;
            std::os::unix::fs::symlink(target, link)
        };
        #[cfg(windows)]
        let made = if dir {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        };
        made.is_ok()
    }

    /// Review of 2026-10-27: Windows reads `.git.`, `.git ` and `.git::$INDEX_ALLOCATION` as `.git`, and the test on
    /// the raw component let `write_file(".git./hooks/post-commit")` through to the real hooks folder.
    #[cfg(windows)]
    #[test]
    fn every_windows_name_of_dot_git_is_read_only() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt");
        std::fs::create_dir_all(root.join(".git").join("hooks")).unwrap();
        for p in [
            ".git./hooks/post-commit",
            ".git /hooks/post-commit",
            ".git::$INDEX_ALLOCATION/hooks/post-commit",
            "sub/.GIT../config",
        ] {
            assert_eq!(
                resolve(&root, &[], p, true).unwrap_err().code,
                "read_only",
                "{p}"
            );
        }
        // The 8.3 name reaches `.git` when the volume has short names; it is refused either way.
        assert!(resolve(&root, &[], "GIT~1/hooks/post-commit", true).is_err());
        for p in ["notes.txt.", "a.txt:hidden", "DOCUME~1/x.md"] {
            assert_eq!(
                resolve(&root, &[], p, true).unwrap_err().code,
                "invalid_path",
                "{p}"
            );
        }
        assert!(resolve(&root, &[], "src/a~b.txt", true).is_ok());
        assert!(
            resolve(&root, &[], "GIT~1/hooks/x", false).is_ok(),
            "reading is fine"
        );
    }

    /// `.git` reached through a link inside the worktree is `.git`.
    #[test]
    fn dot_git_through_a_link_is_read_only() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt");
        std::fs::create_dir_all(root.join(".git").join("hooks")).unwrap();
        if !link(&root.join(".git"), &root.join("g"), true) {
            return;
        }
        assert_eq!(
            resolve(&root, &[], "g/hooks/post-commit", true)
                .unwrap_err()
                .code,
            "read_only"
        );
    }

    /// Review of 2026-10-27: a dangling link counted as a new file inside the worktree (`exists()` follows links), and
    /// `fs::write` through it created its target outside (Linux/macOS).
    #[test]
    fn a_dangling_link_is_not_a_new_file_inside_the_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt");
        std::fs::create_dir_all(&root).unwrap();
        let planted = tmp.path().join("planted.txt");
        if !link(&planted, &root.join("notes.txt"), false) {
            return;
        }
        for write in [true, false] {
            assert_eq!(
                resolve(&root, &[], "notes.txt", write).unwrap_err().code,
                "out_of_scope"
            );
        }
        assert!(!confined(&root.join("notes.txt").join("x"), &root));
        assert!(resolve(&root, &[], "other.txt", true).is_ok());
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
