//! Screenshots a worker attaches to its report (artifacts of kind `screenshot`, ADR-158): checked and copied out of the
//! worktree into `<project>/.agenium/evidence/<KEY>/` (outside git, kept after the worktree goes), shown on the ticket
//! and, through `share_image`, in the Lead's chat with the user.

use std::path::{Path, PathBuf};

use agenium_core::forms::WorkReport;

use crate::{OrchError, Result};

/// The largest screenshot kept (as `view_image` reads).
pub const MAX_BYTES: u64 = 3_750_000;

/// Where `key`'s screenshots are kept in the project at `project_root`.
pub fn dir(project_root: &Path, key: &str) -> PathBuf {
    project_root.join(".agenium").join("evidence").join(key)
}

/// The image types kept, by their first bytes: PNG, JPEG, GIF, WebP.
pub fn media_type(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => None,
    }
}

/// A file name as kept: no folder, nothing that could leave the evidence folder.
pub fn safe_name(name: &str) -> Option<String> {
    let n = name.rsplit(['/', '\\']).next()?.trim();
    (!n.is_empty() && n != "." && n != ".." && !n.contains(':')).then(|| n.to_owned())
}

/// Copies each screenshot artifact of `r` from `worktree` into the evidence folder and points it there (its file name),
/// refusing one that is outside the worktree, missing, too large or not an image, so the worker can fix it.
pub fn keep(project_root: &Path, worktree: &Path, key: &str, r: &mut WorkReport) -> Result<()> {
    for a in r.artifacts.iter_mut().filter(|a| a.kind == "screenshot") {
        let src = worktree.join(a.reference.trim_start_matches(['/', '\\']));
        if !agenium_tools::paths::confined(&src, worktree) {
            return Err(OrchError::Invalid(format!(
                "screenshot {} is not a file of your worktree",
                a.reference
            )));
        }
        let meta = std::fs::metadata(&src).map_err(|_| {
            OrchError::Invalid(format!(
                "screenshot {} does not exist in your worktree",
                a.reference
            ))
        })?;
        if meta.len() > MAX_BYTES {
            return Err(OrchError::Invalid(format!(
                "screenshot {} is {} bytes, more than {MAX_BYTES}: take a smaller one (the part that matters)",
                a.reference,
                meta.len()
            )));
        }
        let bytes =
            std::fs::read(&src).map_err(|e| OrchError::Invalid(format!("{}: {e}", a.reference)))?;
        if media_type(&bytes).is_none() {
            return Err(OrchError::Invalid(format!(
                "screenshot {} is not a PNG, JPEG, GIF or WebP image",
                a.reference
            )));
        }
        let name = safe_name(&a.reference).ok_or_else(|| {
            OrchError::Invalid(format!("screenshot {} has no file name", a.reference))
        })?;
        let to = dir(project_root, key);
        std::fs::create_dir_all(&to)
            .map_err(|e| OrchError::Invalid(format!("{}: {e}", to.display())))?;
        std::fs::write(to.join(&name), &bytes)
            .map_err(|e| OrchError::Invalid(format!("{name}: {e}")))?;
        a.reference = name;
    }
    Ok(())
}

/// A kept screenshot of `key` by its file name, with its media type; `None` when there is none such.
pub fn read(project_root: &Path, key: &str, name: &str) -> Option<(&'static str, Vec<u8>)> {
    let name = safe_name(name)?;
    let bytes = std::fs::read(dir(project_root, key).join(name)).ok()?;
    Some((media_type(&bytes)?, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screenshot_is_kept_out_of_the_worktree_and_a_bad_one_refused() {
        let d = tempfile::tempdir().unwrap();
        let (root, wt) = (
            d.path().join("proj"),
            d.path().join("proj/.agenium/worktrees/AGN-1"),
        );
        std::fs::create_dir_all(wt.join(".agenium/shots")).unwrap();
        let png = [&[0x89, b'P', b'N', b'G', 13, 10, 26, 10][..], &[0u8; 16]].concat();
        std::fs::write(wt.join(".agenium/shots/page.png"), &png).unwrap();
        std::fs::write(wt.join("notes.txt"), "not an image").unwrap();
        let report = |r: &str| {
            WorkReport {
            artifacts: vec![agenium_core::forms::ArtifactRef { kind: "screenshot".into(), reference: r.into() }],
            ..serde_json::from_value(serde_json::json!({"ticket_id": "AGN-1", "summary": "s",
                "acceptance": [{"ac_id": "AC-1", "status": "Met", "evidence": "e"}], "confidence": 0.9})).unwrap()
        }
        };
        let mut r = report(".agenium/shots/page.png");
        keep(&root, &wt, "AGN-1", &mut r).unwrap();
        assert_eq!(r.artifacts[0].reference, "page.png");
        assert_eq!(read(&root, "AGN-1", "page.png").unwrap().0, "image/png");
        assert!(
            read(&root, "AGN-1", "../AGN-1/page.png").is_some_and(|(t, _)| t == "image/png"),
            "the name only"
        );
        assert!(read(&root, "AGN-1", "missing.png").is_none());
        for bad in ["notes.txt", "missing.png", "../../../outside.png"] {
            assert!(
                keep(&root, &wt, "AGN-1", &mut report(bad)).is_err(),
                "{bad}"
            );
        }
    }
}
