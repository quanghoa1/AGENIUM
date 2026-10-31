//! What the OS says about an open file: where it really is and how many names it has (path review of 2026-10-29). The
//! checked write (`real_write.rs`) and the checked read (`real_read.rs`) ask it after the open, which no link swapped in
//! after the path's check can get around.

use std::path::{Path, PathBuf};

/// Opens `p` the way the file tools may: never blocking on a FIFO or a device (a command can leave one in the worktree:
/// `mkfifo`), and only a regular file. The open of a FIFO waited for a writer forever, on an async thread and past the
/// run's stop (audit L3, 2026-10-31). `write` opens it for writing (made where nothing is, with `create_new`).
pub(super) fn open_regular(
    p: &Path,
    write: bool,
    create_new: bool,
) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    if write {
        o.write(true).create_new(create_new);
    } else {
        o.read(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.custom_flags(libc::O_NONBLOCK);
    }
    let f = o.open(p)?;
    if !f.metadata()?.file_type().is_file() {
        return Err(std::io::Error::other(format!(
            "{} is not a regular file (a pipe or a device)",
            p.display()
        )));
    }
    Ok(f)
}

/// The bytes of `p` read through [`open_regular`].
pub(super) fn read_regular(p: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut b = Vec::new();
    open_regular(p, false, false)?.read_to_end(&mut b)?;
    Ok(b)
}

/// Where an open file really is; `None` where the OS cannot say (the path was checked before the open).
#[cfg(windows)]
pub(super) fn real_path(f: &std::fs::File) -> Option<PathBuf> {
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
pub(super) fn real_path(f: &std::fs::File) -> Option<PathBuf> {
    use std::os::unix::io::AsRawFd;
    std::fs::read_link(format!("/proc/self/fd/{}", f.as_raw_fd())).ok()
}

/// macOS run 6 (2026-10-30): without this the checks after the open were skipped there.
#[cfg(target_os = "macos")]
pub(super) fn real_path(f: &std::fs::File) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::AsRawFd;
    let mut buf = [0u8; libc::MAXPATHLEN as usize];
    // SAFETY: F_GETPATH writes a NUL-terminated path of at most MAXPATHLEN bytes into the buffer.
    if unsafe { libc::fcntl(f.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) } == -1 {
        return None;
    }
    let n = buf.iter().position(|&b| b == 0)?;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..n])))
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub(super) fn real_path(_: &std::fs::File) -> Option<PathBuf> {
    None
}

/// How many names the open file has.
#[cfg(windows)]
pub(super) fn links(f: &std::fs::File) -> u32 {
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
pub(super) fn links(f: &std::fs::File) -> u32 {
    use std::os::unix::fs::MetadataExt;
    f.metadata().map(|m| m.nlink() as u32).unwrap_or(1)
}
