//! Writing a file where it really is (path review of 2026-10-29). Agenium's tools run outside the sandbox, and a command
//! inside it can turn a folder of the worktree into a link to anywhere between the check of a path and the write to
//! it. So the file is opened first, then the OS is asked where the open file really is: it is written only inside the
//! worktree and when it has no other name (a hard link would carry the write to its other names).

use std::io::Write;
use std::path::Path;

use super::handle::{links, real_path};
use super::*;
use crate::paths;

/// Writes `content` to `p` (a path already checked against `root`), refusing a file that the open handle shows outside
/// `root` or with other names.
pub(super) fn write_file(
    root: &Path,
    p: &Path,
    rel: &str,
    content: &[u8],
) -> Result<(), ToolError> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| io_err(rel, e))?;
    }
    let existed = std::fs::symlink_metadata(p).is_ok();
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(p)
        .map_err(|e| io_err(rel, e))?;
    let canon_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let real = real_path(&f);
    if let Some(real) = real.as_ref().filter(|r| !paths::within(r, &canon_root)) {
        drop(f);
        // The empty file the open made out there goes again (by where it is, not by the name that led there).
        if !existed {
            let _ = std::fs::remove_file(real);
        }
        return Err(ToolError::new(
            "out_of_scope",
            format!("{rel} leads outside your worktree (a link changed under it)"),
        ));
    }
    if links(&f) > 1 {
        return Err(ToolError::new(
            "out_of_scope",
            format!("{rel} has other names (hard links): a write would change them too"),
        ));
    }
    f.set_len(0).map_err(|e| io_err(rel, e))?;
    f.write_all(content).map_err(|e| io_err(rel, e))
}
