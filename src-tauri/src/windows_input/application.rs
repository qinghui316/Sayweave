//! Application metadata from a captured process, independent of input admission.
use super::Anchor;
use crate::contracts::language::ApplicationContext;
use windows::Win32::{Foundation::*, System::Threading::*};
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetPackageFamilyName(process: HANDLE, length: *mut u32, name: *mut u16) -> i32;
    fn QueryFullProcessImageNameW(
        process: HANDLE,
        flags: u32,
        name: *mut u16,
        length: *mut u32,
    ) -> i32;
}
pub(super) fn resolve(anchor: &Anchor) -> ApplicationContext {
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, anchor.process)
        else {
            return ApplicationContext::default();
        };
        struct Handle(HANDLE);
        impl Drop for Handle {
            fn drop(&mut self) {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
        let process = Handle(process);
        let same_lifetime = || {
            let (mut created, mut exit, mut kernel, mut user) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            GetProcessTimes(process.0, &mut created, &mut exit, &mut kernel, &mut user).is_ok()
                && ((created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64)
                    == anchor.lifetime
        };
        if !same_lifetime() {
            return ApplicationContext::default();
        }
        let mut length = 32768;
        let mut path = vec![0u16; length as usize];
        if QueryFullProcessImageNameW(process.0, 0, path.as_mut_ptr(), &mut length) == 0 {
            return ApplicationContext::default();
        }
        let executable = String::from_utf16_lossy(&path[..length as usize]);
        let mut length = 0;
        let family = if GetPackageFamilyName(process.0, &mut length, std::ptr::null_mut()) == 122
            && length > 0
            && length < 4096
        {
            let mut name = vec![0u16; length as usize];
            if GetPackageFamilyName(process.0, &mut length, name.as_mut_ptr()) == 0 {
                Some(String::from_utf16_lossy(
                    &name[..length.saturating_sub(1) as usize],
                ))
            } else {
                None
            }
        } else {
            None
        };
        if !same_lifetime() {
            return ApplicationContext::default();
        }
        ApplicationContext::from_identity(family.as_deref(), &executable)
    }
}
