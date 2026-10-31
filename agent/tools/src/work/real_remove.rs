//! Deleting a file where it really is (review of 2026-10-29, the delete side of `real_write.rs`). `apply_patch` deletes
//! and moves files, and undoes its own writes when one fails. Deleting by the checked path let a command in the sandbox
//! swap a folder of the worktree for a link between the check and the delete, and Agenium (outside the sandbox) deleted
//! a file out there. The delete goes through what was opened and checked: on Windows the file's own handle, on Linux
//! and macOS the folder's (`unlinkat` of one name in it).

use std::path::Path;

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
use super::handle::real_path;
use super::*;
use crate::paths;

fn moved(rel: &str) -> ToolError {
    ToolError::new(
        "out_of_scope",
        format!("{rel} leads outside your worktree (a link changed under it)"),
    )
}

fn inside(root: &Path, real: &Path) -> bool {
    paths::within(
        real,
        &std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()),
    )
}

/// Deletes the file `p` (a path already checked against `root`) only where it really is inside `root`.
#[cfg(windows)]
pub(super) fn remove_file(root: &Path, p: &Path, rel: &str) -> Result<(), ToolError> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
    };
    // DELETE | FILE_READ_ATTRIBUTES; a link at the end is opened itself, not followed (FILE_FLAG_OPEN_REPARSE_POINT).
    let f = std::fs::OpenOptions::new()
        .access_mode(0x0001_0000 | 0x80)
        .custom_flags(0x0020_0000)
        .open(p)
        .map_err(|e| io_err(rel, e))?;
    match real_path(&f) {
        Some(real) if inside(root, &real) => {}
        _ => return Err(moved(rel)),
    }
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: the handle is the open file's; the struct and its size are passed together.
    let ok = unsafe {
        SetFileInformationByHandle(
            f.as_raw_handle() as _,
            FileDispositionInfo,
            &info as *const _ as *const _,
            std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(io_err(rel, std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Deletes the file `p` (a path already checked against `root`) only where it really is inside `root`.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn remove_file(root: &Path, p: &Path, rel: &str) -> Result<(), ToolError> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::AsRawFd;
    let (Some(dir), Some(name)) = (p.parent(), p.file_name()) else {
        return Err(io_err(rel, std::io::Error::other("no file name")));
    };
    let d = std::fs::File::open(dir).map_err(|e| io_err(rel, e))?;
    match real_path(&d) {
        Some(real) if inside(root, &real) => {}
        _ => return Err(moved(rel)),
    }
    let name = std::ffi::CString::new(name.as_bytes())
        .map_err(|e| io_err(rel, std::io::Error::other(e)))?;
    // SAFETY: a folder descriptor held open above and a NUL-terminated name.
    if unsafe { libc::unlinkat(d.as_raw_fd(), name.as_ptr(), 0) } != 0 {
        return Err(io_err(rel, std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Deletes the file `p` (a path already checked against `root`): its folder is checked, then the name deleted (no
/// open-handle path here, as for writes).
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub(super) fn remove_file(root: &Path, p: &Path, rel: &str) -> Result<(), ToolError> {
    let dir = p.parent().and_then(|d| std::fs::canonicalize(d).ok());
    if !dir.is_some_and(|d| inside(root, &d)) {
        return Err(moved(rel));
    }
    std::fs::remove_file(p).map_err(|e| io_err(rel, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `link` becomes a link to `target` (a junction on Windows, which needs no privilege).
    fn link_dir(target: &Path, link: &Path) -> bool {
        #[cfg(windows)]
        {
            let arg = |p: &Path| p.display().to_string().replace('/', "\\");
            std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J", &arg(link), &arg(target)])
                .output()
                .is_ok_and(|o| o.status.success())
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
    }

    /// Review of 2026-10-29: a folder of the worktree swapped for a link after the path's check. Neither a delete nor
    /// a write reaches the user's file out there, and nothing out there is deleted.
    #[test]
    fn a_folder_swapped_for_a_link_keeps_deletes_and_writes_inside() {
        let (t, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let root = t.path().join("wt");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(out.path().join(".bashrc"), "mine").unwrap();
        if !link_dir(out.path(), &root.join("sub")) {
            eprintln!("no link here: skipped");
            return;
        }
        let e = remove_file(&root, &root.join("sub/.bashrc"), "sub/.bashrc").unwrap_err();
        assert_eq!(e.code, "out_of_scope", "{e}");
        let e = super::super::write_file(&root, &root.join("sub/.bashrc"), "sub/.bashrc", b"x")
            .unwrap_err();
        assert_eq!(e.code, "out_of_scope", "{e}");
        assert_eq!(
            std::fs::read_to_string(out.path().join(".bashrc")).unwrap(),
            "mine"
        );
        // A new name out there: made empty and left, never a file that was there.
        let e = super::super::write_file(&root, &root.join("sub/new.txt"), "sub/new.txt", b"x")
            .unwrap_err();
        assert!(e.message.contains("empty file was left"), "{e}");
        assert_eq!(std::fs::read(out.path().join("new.txt")).unwrap(), b"");
        // Inside the worktree a delete still works.
        std::fs::write(root.join("mine.txt"), "x").unwrap();
        remove_file(&root, &root.join("mine.txt"), "mine.txt").unwrap();
        assert!(!root.join("mine.txt").exists());
        let _ = std::fs::remove_dir(root.join("sub"));
    }
}
