//! A private child job; the process cannot outlive the application handle.
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::HANDLE,
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
    },
    core::PCWSTR,
};

#[cfg(windows)]
pub struct Job(OwnedHandle);
#[cfg(not(windows))]
pub struct Job;

impl Job {
    #[cfg(windows)]
    pub fn attach(child: &tokio::process::Child) -> Result<Self, &'static str> {
        // SAFETY: the unnamed job has default security and its returned owned
        // handle is immediately placed in OwnedHandle for every error path.
        let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
            .map_err(|_| "localProcessUnavailable")?;
        let owned = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: info has the exact documented size; child is still owned/live.
        unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of_val(&info) as u32,
            )
            .map_err(|_| "localProcessUnavailable")?;
            AssignProcessToJobObject(
                handle,
                HANDLE(child.raw_handle().ok_or("localProcessUnavailable")?),
            )
            .map_err(|_| "localProcessUnavailable")?;
        }
        Ok(Self(owned))
    }
    #[cfg(not(windows))]
    pub fn attach(_: &tokio::process::Child) -> Result<Self, &'static str> {
        Err("localPlatformUnsupported")
    }

    #[cfg(windows)]
    pub fn terminate(&self) {
        // SAFETY: this private job handle remains owned for the call.
        let _ = unsafe { TerminateJobObject(HANDLE(self.0.as_raw_handle()), 1) };
    }
    #[cfg(not(windows))]
    pub fn terminate(&self) {}
}
