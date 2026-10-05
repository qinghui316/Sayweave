//! Ephemeral host discovery shared by ordinary and packaged target processes.
//! HKCU can have different views in those processes and is not an IPC channel.
use crate::transport::{Pipe, current_user_sid, identity, session};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        System::Memory::*,
    },
    core::PCWSTR,
};

const SIZE: usize = 256;
const READY_V1: u32 = 0x53574831;
pub(crate) struct UserSecurity(PSECURITY_DESCRIPTOR);
impl UserSecurity {
    pub(crate) fn new() -> windows::core::Result<Self> {
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{})", current_user_sid()?)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )?;
        }
        Ok(Self(descriptor))
    }
    pub(crate) fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0.0,
            bInheritHandle: false.into(),
        }
    }
}
impl Drop for UserSecurity {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0.0)));
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Record {
    pid: u32,
    session: u32,
    created: u64,
    capability: String,
}
fn name() -> windows::core::Result<Vec<u16>> {
    Ok(format!(
        "Local\\Sayweave.TSF.HostIdentity.v1.{}",
        current_user_sid()?
    )
    .encode_utf16()
    .chain(Some(0))
    .collect())
}
struct View(MEMORY_MAPPED_VIEW_ADDRESS);
impl View {
    fn open(handle: HANDLE, access: FILE_MAP) -> windows::core::Result<Self> {
        let view = unsafe { MapViewOfFile(handle, access, 0, 0, SIZE) };
        if view.Value.is_null() {
            Err(windows::core::Error::from_thread())
        } else {
            Ok(Self(view))
        }
    }
    fn state(&self) -> &AtomicU32 {
        // Mapped views are page aligned; the first word is the publication flag.
        unsafe { &*self.0.Value.cast::<AtomicU32>() }
    }
}
impl Drop for View {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.0);
        }
    }
}
pub(crate) struct Publication {
    view: View,
    _handle: Pipe,
}
impl Publication {
    pub(crate) fn new(pid: u32, capability: &str) -> windows::core::Result<Self> {
        Self::create(&name()?, pid, capability)
    }
    fn create(name: &[u16], pid: u32, capability: &str) -> windows::core::Result<Self> {
        if pid != std::process::id() || uuid::Uuid::parse_str(capability).is_err() {
            return Err(windows::core::Error::from_hresult(E_INVALIDARG));
        }
        let peer = identity(pid)?;
        let data = serde_json::to_vec(&Record {
            pid,
            session: peer.session,
            created: peer.created,
            capability: capability.into(),
        })
        .map_err(|_| windows::core::Error::from_hresult(E_INVALIDARG))?;
        if data.len() > SIZE - 8 {
            return Err(windows::core::Error::from_hresult(E_INVALIDARG));
        }
        unsafe {
            let security = UserSecurity::new()?;
            let attributes = security.attributes();
            SetLastError(ERROR_SUCCESS);
            let handle = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                Some(&attributes),
                PAGE_READWRITE,
                0,
                SIZE as u32,
                PCWSTR(name.as_ptr()),
            );
            let existed = GetLastError() == ERROR_ALREADY_EXISTS;
            let handle = Pipe(handle?);
            // An old reader or a pre-created object must not be overwritten or trusted.
            if existed {
                return Err(windows::core::Error::new(
                    E_ACCESSDENIED,
                    "hostIdentityAlreadyExists",
                ));
            }
            let view = View::open(handle.0, FILE_MAP_WRITE)?;
            let base = view.0.Value.cast::<u8>();
            std::ptr::write(base.add(4).cast::<u32>(), data.len() as u32);
            std::ptr::copy_nonoverlapping(data.as_ptr(), base.add(8), data.len());
            view.state().store(READY_V1, Ordering::Release);
            Ok(Self {
                view,
                _handle: handle,
            })
        }
    }
}
impl Drop for Publication {
    fn drop(&mut self) {
        // Invalidate even when a reader still holds a mapping handle. The immutable
        // record is never reused, and the owner mutex is released only after this drop.
        self.view.state().store(0, Ordering::Release);
    }
}
pub(crate) fn read() -> Option<(u32, String)> {
    read_named(&name().ok()?)
}
fn read_named(name: &[u16]) -> Option<(u32, String)> {
    unsafe {
        let handle = Pipe(OpenFileMappingW(FILE_MAP_READ.0, false, PCWSTR(name.as_ptr())).ok()?);
        let view = View::open(handle.0, FILE_MAP_READ).ok()?;
        if view.state().load(Ordering::Acquire) != READY_V1 {
            return None;
        }
        let base = view.0.Value.cast::<u8>();
        let length = std::ptr::read(base.add(4).cast::<u32>()) as usize;
        if length == 0 || length > SIZE - 8 {
            return None;
        }
        let record: Record =
            serde_json::from_slice(std::slice::from_raw_parts(base.add(8), length)).ok()?;
        let peer = identity(record.pid).ok()?;
        if view.state().load(Ordering::Acquire) != READY_V1
            || peer.session != record.session
            || peer.created != record.created
            || record.session != session(std::process::id()).ok()?
            || uuid::Uuid::parse_str(&record.capability).is_err()
        {
            return None;
        }
        Some((record.pid, record.capability))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_identity_is_immutable_and_retained_reader_cannot_keep_a_lease_alive() {
        let name: Vec<_> = format!("Local\\Sayweave.TSF.Test.{}", uuid::Uuid::new_v4())
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let capability = uuid::Uuid::new_v4().to_string();
        let host = Publication::create(&name, std::process::id(), &capability).unwrap();
        assert_eq!(
            read_named(&name),
            Some((std::process::id(), capability.clone()))
        );
        let reader = unsafe {
            Pipe(OpenFileMappingW(FILE_MAP_READ.0, false, PCWSTR(name.as_ptr())).unwrap())
        };
        assert!(Publication::create(&name, std::process::id(), &capability).is_err());
        drop(host);
        assert_eq!(read_named(&name), None);
        assert!(Publication::create(&name, std::process::id(), &capability).is_err());
        drop(reader);
        let next = uuid::Uuid::new_v4().to_string();
        let host = Publication::create(&name, std::process::id(), &next).unwrap();
        assert_eq!(read_named(&name), Some((std::process::id(), next)));
        drop(host);
        assert_eq!(read_named(&name), None);
    }
}
