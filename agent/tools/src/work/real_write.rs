//! Writing a file where it really is (path review of 2026-10-29). Agenium's tools run outside the sandbox, and a command
//! inside it can turn a folder of the worktree into a link to anywhere between the check of a path and the write to
//! it. So the file is opened first, then the OS is asked where the open file really is: it is written only inside the
//! worktree and when it has no other name (a hard link would carry the write to its other names).

use std::io::Write;
use std::path::{Path, PathBuf};

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

/// Where an open file really is; `None` where the OS cannot say (the path was checked before the open).
#[cfg(windows)]
fn real_path(f: &std::fs::File) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
    let mut buf = vec![0u16; 1024];
    loop {
        // SAFETY: the handle is the open file's; the buffer's length is passed with it.
        let n = unsafe {
            GetFinalPathNameByHandleW(
                f.as_raw_handle() as _,
                buf.as_mut_ptr(),
                buf.len() as u32,
                0,
            )
        } as usize;
        if n == 0 {
            return None;
        }
        if n < buf.len() {
            return Some(PathBuf::from(std::ffi::OsString::from_wide(&buf[..n])));
        }
        buf.resize(n + 1, 0);
    }
}

#[cfg(target_os = "linux")]
fn real_path(f: &std::fs::File) -> Option<PathBuf> {
    use std::os::unix::io::AsRawFd;
    std::fs::read_link(format!("/proc/self/fd/{}", f.as_raw_fd())).ok()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn real_path(_: &std::fs::File) -> Option<PathBuf> {
    None
}

/// How many names the open file has.
#[cfg(windows)]
fn links(f: &std::fs::File) -> u32 {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    // SAFETY: a zeroed plain-data struct the call fills; the handle is the open file's.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(f.as_raw_handle() as _, &mut info) } == 0 {
        return 1;
    }
    info.nNumberOfLinks
}

#[cfg(unix)]
fn links(f: &std::fs::File) -> u32 {
    use std::os::unix::fs::MetadataExt;
    f.metadata().map(|m| m.nlink() as u32).unwrap_or(1)
}
