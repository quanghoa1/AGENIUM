//! A command's whole process tree, ended together (a Job object on Windows, the process group elsewhere).

use super::*;

/// Kills a child's whole process tree when dropped (Windows: job object; Unix: its process group).
pub struct TreeGuard {
    #[cfg(windows)]
    pub(super) _job: Option<win_job::Job>,
    #[cfg(unix)]
    pub(super) pgid: Option<u32>,
}

impl TreeGuard {
    pub fn attach(child: &tokio::process::Child) -> TreeGuard {
        #[cfg(windows)]
        {
            let job = win_job::Job::new()
                .and_then(|j| {
                    child
                        .raw_handle()
                        .map(|h| j.assign(h))
                        .unwrap_or(Ok(()))
                        .map(|_| j)
                })
                .ok();
            TreeGuard { _job: job }
        }
        #[cfg(unix)]
        {
            TreeGuard { pgid: child.id() }
        }
    }
}

impl Drop for TreeGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pgid) = self.pgid {
            let _ = std::process::Command::new("kill")
                .args(["-KILL", "--", &format!("-{pgid}")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

#[cfg(windows)]
pub(super) mod win_job {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub struct Job(HANDLE);

    // SAFETY: a job handle may be used from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn new() -> std::io::Result<Job> {
            // SAFETY: plain Win32 calls with valid arguments; the handle is owned by `Job` and closed in Drop.
            unsafe {
                let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if h.is_null() {
                    return Err(std::io::Error::last_os_error());
                }
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 {
                    let e = std::io::Error::last_os_error();
                    CloseHandle(h);
                    return Err(e);
                }
                Ok(Job(h))
            }
        }

        pub fn assign(&self, process: std::os::windows::io::RawHandle) -> std::io::Result<()> {
            // SAFETY: `process` is a live process handle owned by the tokio Child.
            if unsafe { AssignProcessToJobObject(self.0, process as HANDLE) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }

        pub fn terminate(&self) {
            // SAFETY: valid job handle.
            unsafe { TerminateJobObject(self.0, 1) };
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: closing the last handle kills the remaining processes (KILL_ON_JOB_CLOSE).
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// Kills a process and all its descendants.
pub async fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = tokio::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW: the desktop app has no console
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    #[cfg(unix)]
    {
        let _ = tokio::process::Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .status()
            .await;
    }
}
