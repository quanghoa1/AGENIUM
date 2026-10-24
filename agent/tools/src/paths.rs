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
    let canon_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let in_root = within(&norm, root) && within(&real, &canon_root);
    let in_read_root = !write
        && read_roots.iter().any(|r| {
            let cr = std::fs::canonicalize(r).unwrap_or_else(|_| r.clone());
            within(&norm, r) && within(&real, &cr)
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
