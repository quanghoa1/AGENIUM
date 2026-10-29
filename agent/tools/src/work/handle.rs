//! What the OS says about an open file: where it really is and how many names it has (path review of 2026-10-29). The
//! checked write (`real_write.rs`) and the checked read (`real_read.rs`) ask it after the open, which no link swapped in
//! after the path's check can get around.

use std::path::PathBuf;

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

#[cfg(not(any(windows, target_os = "linux")))]
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
