//! Process-tree ownership for Windows.
//!
//! `TerminateProcess` only stops the selected process. Our sidecars launch
//! Python/tidevice descendants, so the desktop root must own a kill-on-close
//! Job Object for Windows to release every USB relay after a crash or forced
//! shutdown.

/// Install one process-lifetime guard for the current process.
///
/// On Windows the guard assigns this process to a Job Object configured with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. All descendants inherit membership.
/// Other platforms already propagate the signals used by the supervisor.
pub fn install_process_tree_guard() -> anyhow::Result<()> {
    install_platform_guard()
}

#[cfg(not(windows))]
fn install_platform_guard() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn install_platform_guard() -> anyhow::Result<()> {
    use std::sync::OnceLock;

    static PROCESS_JOB: OnceLock<Result<KillOnCloseJob, String>> = OnceLock::new();
    match PROCESS_JOB.get_or_init(|| {
        let job = KillOnCloseJob::create().map_err(|error| error.to_string())?;
        job.assign_current_process()
            .map_err(|error| error.to_string())?;
        Ok(job)
    }) {
        Ok(_) => Ok(()),
        Err(error) => anyhow::bail!("cannot establish Windows process-tree ownership: {error}"),
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct KillOnCloseJob {
    handle: usize,
}

#[cfg(windows)]
impl KillOnCloseJob {
    fn create() -> anyhow::Result<Self> {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        // SAFETY: null security/name pointers request an unnamed Job Object
        // with default security. The returned handle is checked before use.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            let code = unsafe { GetLastError() };
            anyhow::bail!("CreateJobObjectW failed with Win32 error {code}");
        }

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `limits` has exactly the structure and size required by the
        // selected information class and remains alive for the whole call.
        let configured = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            let code = unsafe { GetLastError() };
            unsafe { CloseHandle(handle) };
            anyhow::bail!("SetInformationJobObject failed with Win32 error {code}");
        }

        Ok(Self {
            handle: handle as usize,
        })
    }

    fn assign_current_process(&self) -> anyhow::Result<()> {
        use windows_sys::Win32::System::Threading::GetCurrentProcess;

        // SAFETY: GetCurrentProcess returns a valid pseudo-handle for the
        // lifetime of this process.
        self.assign_handle(unsafe { GetCurrentProcess() })
    }

    #[cfg(test)]
    fn assign_raw_handle(&self, handle: std::os::windows::io::RawHandle) -> anyhow::Result<()> {
        self.assign_handle(handle.cast())
    }

    fn assign_handle(&self, process: windows_sys::Win32::Foundation::HANDLE) -> anyhow::Result<()> {
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

        // SAFETY: both handles are live kernel handles. Membership is applied
        // synchronously and neither handle is closed during the call.
        if unsafe { AssignProcessToJobObject(self.handle as _, process) } == 0 {
            let code = unsafe { GetLastError() };
            anyhow::bail!("AssignProcessToJobObject failed with Win32 error {code}");
        }
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for KillOnCloseJob {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;

        // SAFETY: this type exclusively owns the handle returned by
        // CreateJobObjectW and closes it exactly once.
        unsafe {
            CloseHandle(self.handle as _);
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use std::time::Duration;

    #[test]
    fn closing_job_terminates_an_assigned_process() {
        let mut child = std::process::Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ])
            .spawn()
            .expect("spawn sleeping fixture");

        let job = KillOnCloseJob::create().expect("create kill-on-close job");
        job.assign_raw_handle(child.as_raw_handle())
            .expect("assign fixture to job");
        drop(job);

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if child.try_wait().expect("query fixture status").is_some() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "assigned process survived closing the job"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
