//! Reading a file where it really is (path review of 2026-10-29, the read side of `real_write.rs`). The path was checked,
//! but a command can swap a folder of the worktree for a link between the check and the read, and a hard link inside
//! the worktree can be another name of a file anywhere on the disk. So the file is opened first, then read only when
//! the open handle is inside the run's roots and the file has no other name. A file with other names can still be read
//! with `shell`, which runs where the sandbox decides what is reachable.

use std::io::Read;
use std::path::Path;

use super::handle::{links, open_regular, real_path};
use super::*;
use crate::paths;

/// The bytes of `p` (a path already checked), at most `max` of them; `rel` names it in errors.
pub(super) fn read_checked(
    ctx: &ToolCtx,
    p: &Path,
    rel: &str,
    max: u64,
) -> Result<Vec<u8>, ToolError> {
    let meta = std::fs::metadata(p).map_err(|e| io_err(rel, e))?;
    if meta.is_dir() {
        return Err(ToolError::new(
            "is_directory",
            format!("{rel} is a directory; use list_dir"),
        ));
    }
    let f = open_regular(p, false, false).map_err(|e| io_err(rel, e))?;
    let len = f.metadata().map(|m| m.len()).unwrap_or(meta.len());
    if len > max {
        return Err(ToolError::new(
            "too_large",
            format!("{rel} is {len} bytes; use grep or read a range"),
        ));
    }
    if let Some(real) = real_path(&f) {
        let inside = std::iter::once(&ctx.root).chain(&ctx.read_roots).any(|r| {
            paths::within(
                &real,
                &std::fs::canonicalize(r).unwrap_or_else(|_| r.clone()),
            )
        });
        if !inside {
            return Err(ToolError::new(
                "out_of_scope",
                format!("{rel} leads outside your worktree (a link changed under it)"),
            ));
        }
    }
    if links(&f) > 1 {
        return Err(ToolError::new(
            "out_of_scope",
            format!(
                "{rel} has other names (hard links), which may be files outside your worktree: read it with shell if \
                 you need it"
            ),
        ));
    }
    let mut bytes = Vec::with_capacity(len as usize);
    f.take(max)
        .read_to_end(&mut bytes)
        .map_err(|e| io_err(rel, e))?;
    Ok(bytes)
}
