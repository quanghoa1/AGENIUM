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
    // A file that was there is opened, never made; a new one is made only where nothing is (`create_new`): a folder
    // swapped for a link in between cannot turn the write into one on a file that exists out there.
    let existed = std::fs::symlink_metadata(p).is_ok();
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(!existed)
        .open(p)
        .map_err(|e| io_err(rel, e))?;
    let canon_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let real = real_path(&f);
    if let Some(real) = real.as_ref().filter(|r| !paths::within(r, &canon_root)) {
        // Nothing out there is deleted: an empty file this call made there stays (deleting by its path could reach
        // another file by then); the user is told where.
        let made = if existed {
            String::new()
        } else {
            format!(" (an empty file was left at {})", real.display())
        };
        return Err(ToolError::new(
            "out_of_scope",
            format!("{rel} leads outside your worktree (a link changed under it){made}"),
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
