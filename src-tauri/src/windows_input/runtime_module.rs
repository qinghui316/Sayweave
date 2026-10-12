//! Verify the component actually mapped in the authenticated input target.
//! A registration update cannot replace a DLL already loaded by another process.
use crate::history::types::Error;
use std::path::{Path, PathBuf};

fn unavailable() -> Error {
    Error::new("compositionRuntimeUnavailable")
}
fn verify_files(modules: &[PathBuf], registered: &Path, payload: &[u8]) -> Result<(), Error> {
    // Multiple same-name modules cannot be attributed to the Inspect reply.
    let [module] = modules else {
        return Err(unavailable());
    };
    let base = registered
        .parent()
        .and_then(Path::parent)
        .ok_or_else(unavailable)?;
    let parent = module.parent().ok_or_else(unavailable)?;
    if !parent.parent().is_some_and(|p| {
        p.to_string_lossy()
            .eq_ignore_ascii_case(&base.to_string_lossy())
    }) || !parent.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit())
    }) {
        return Err(unavailable());
    }
    let metadata = std::fs::symlink_metadata(module).map_err(|_| unavailable())?;
    use std::os::windows::fs::MetadataExt;
    let directory = std::fs::symlink_metadata(parent).map_err(|_| unavailable())?;
    if !metadata.is_file()
        || metadata.file_attributes() & 0x400 != 0
        || !directory.is_dir()
        || directory.file_attributes() & 0x400 != 0
    {
        return Err(unavailable());
    }
    use sha2::{Digest, Sha256};
    if !parent.file_name().is_some_and(|name| {
        name.to_string_lossy()
            .eq_ignore_ascii_case(&format!("{:x}", Sha256::digest(payload)))
    }) {
        return Err(Error::new("compositionTargetVersion"));
    }
    if payload.is_empty() || metadata.len() != payload.len() as u64 {
        return Err(Error::new("compositionTargetVersion"));
    }
    use std::io::Read;
    let mut bytes = Vec::with_capacity(payload.len());
    std::fs::File::open(module)
        .map_err(|_| unavailable())?
        .take(payload.len() as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable())?;
    if bytes != payload {
        return Err(Error::new("compositionTargetVersion"));
    }
    Ok(())
}

pub(super) fn verify(process: u32) -> Result<(), Error> {
    use windows::Win32::{Foundation::*, System::Diagnostics::ToolHelp::*};
    struct Snapshot(HANDLE);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    // Module loading can transiently invalidate Toolhelp snapshots. Retry only
    // ERROR_BAD_LENGTH, before any Begin or input write, within a small bound.
    let mut snapshot = None;
    for attempt in 0..3 {
        match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, process) }
        {
            Ok(handle) => {
                snapshot = Some(Snapshot(handle));
                break;
            }
            Err(error)
                if error.code() == windows::core::HRESULT::from_win32(ERROR_BAD_LENGTH.0)
                    && attempt < 2 =>
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(_) => return Err(unavailable()),
        }
    }
    let snapshot = snapshot.ok_or_else(unavailable)?;
    let mut entry = MODULEENTRY32W {
        dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };
    unsafe { Module32FirstW(snapshot.0, &mut entry) }.map_err(|_| unavailable())?;
    let mut modules = Vec::new();
    for count in 0..4096 {
        let name_len = entry
            .szModule
            .iter()
            .position(|c| *c == 0)
            .ok_or_else(unavailable)?;
        let name = String::from_utf16(&entry.szModule[..name_len]).map_err(|_| unavailable())?;
        if name.eq_ignore_ascii_case("sayweave_text_service.dll") {
            let length = entry
                .szExePath
                .iter()
                .position(|c| *c == 0)
                .ok_or_else(unavailable)?;
            modules.push(PathBuf::from(
                String::from_utf16(&entry.szExePath[..length]).map_err(|_| unavailable())?,
            ));
        }
        match unsafe { Module32NextW(snapshot.0, &mut entry) } {
            Ok(()) if count < 4095 => {}
            Ok(()) => return Err(unavailable()),
            Err(error)
                if error.code() == windows::core::HRESULT::from_win32(ERROR_NO_MORE_FILES.0) =>
            {
                break;
            }
            Err(_) => return Err(unavailable()),
        }
    }
    let registered = sayweave_text_service::registration::owned_module()
        .map_err(|_| unavailable())?
        .ok_or_else(unavailable)?;
    let result = verify_files(
        &modules,
        Path::new(&registered),
        crate::text_service::payload(),
    );
    log::info!(target:"sayweave", "operation=draft_payload process={} modules={} result={}", process, modules.len(), result.as_ref().err().map_or("matched", |e|e.code.as_str()));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_protocol_old_bytes_are_rejected_even_with_new_registration() {
        use sha2::{Digest, Sha256};
        let root = tempfile::tempdir().unwrap();
        let old = root
            .path()
            .join(format!("{:x}", Sha256::digest(b"old8")))
            .join("sayweave_text_service.dll");
        let new = root
            .path()
            .join(format!("{:x}", Sha256::digest(b"new8")))
            .join("sayweave_text_service.dll");
        std::fs::create_dir(old.parent().unwrap()).unwrap();
        std::fs::create_dir(new.parent().unwrap()).unwrap();
        std::fs::write(&old, b"old8").unwrap();
        std::fs::write(&new, b"new8").unwrap();
        assert_eq!(
            verify_files(&[old], &new, b"new8").unwrap_err().code,
            "compositionTargetVersion"
        );
        assert!(verify_files(std::slice::from_ref(&new), &new, b"new8").is_ok());
        // A plausible immutable path alone is not evidence of matching bytes.
        std::fs::write(&new, b"bad8").unwrap();
        assert_eq!(
            verify_files(std::slice::from_ref(&new), &new, b"new8")
                .unwrap_err()
                .code,
            "compositionTargetVersion"
        );
        assert_eq!(
            verify_files(&[], &new, b"new8").unwrap_err().code,
            "compositionRuntimeUnavailable"
        );
        assert_eq!(
            verify_files(&[new.clone(), new.clone()], &new, b"new8")
                .unwrap_err()
                .code,
            "compositionRuntimeUnavailable"
        );
    }
    #[test]
    fn unknown_module_location_cannot_supply_matching_payload() {
        let root = tempfile::tempdir().unwrap();
        let unexpected = root.path().join("sayweave_text_service.dll");
        std::fs::write(&unexpected, b"new8").unwrap();
        let registered = root
            .path()
            .join("2".repeat(64))
            .join("sayweave_text_service.dll");
        assert_eq!(
            verify_files(&[unexpected], &registered, b"new8")
                .unwrap_err()
                .code,
            "compositionRuntimeUnavailable"
        );
    }
}
