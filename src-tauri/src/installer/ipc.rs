//! A bounded installer-only pipe; the TSF host transport and its ACL are unchanged.
use super::{
    machine::MachineRecord,
    record::{MAX_FILE, Process, Result, hash, read},
};
use sayweave_text_service::transport::{Pipe, identity, process_user_sid};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        Storage::FileSystem::*,
        System::{Pipes::*, Threading::*},
        UI::Shell::*,
    },
    core::{PCWSTR, PWSTR},
};

pub fn process(pid: u32) -> Result<Process> {
    let id = identity(pid).map_err(|_| "installerProcessIdentity")?;
    let path = process_path(pid)?;
    Ok(Process {
        pid,
        session: id.session,
        created: id.created,
        sid: process_user_sid(pid).map_err(|_| "installerProcessIdentity")?,
        image_hash: hash(&read(&path, MAX_FILE)?),
    })
}
pub fn process_path(pid: u32) -> Result<PathBuf> {
    unsafe {
        let handle = Pipe(
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
                .map_err(|_| "installerProcessIdentity")?,
        );
        let mut buffer = vec![0u16; 32768];
        let mut len = buffer.len() as u32;
        QueryFullProcessImageNameW(
            handle.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
        .map_err(|_| "installerProcessIdentity")?;
        Ok(PathBuf::from(
            String::from_utf16(&buffer[..len as usize]).map_err(|_| "installerProcessIdentity")?,
        ))
    }
}
pub fn live(process: &Process) -> bool {
    identity(process.pid)
        .is_ok_and(|v| v.created == process.created && v.session == process.session)
}
pub fn allow_initiator_query(sid: &str) -> Result<()> {
    unsafe {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let text =
            super::metadata::wide(&format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x101000;;;{sid})"));
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(text.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .map_err(|_| "installerProcessSecurity")?;
        let result =
            SetKernelObjectSecurity(GetCurrentProcess(), DACL_SECURITY_INFORMATION, descriptor)
                .map_err(|_| "installerProcessSecurity".into());
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        result
    }
}
fn name(transaction: &str) -> Result<Vec<u16>> {
    if uuid::Uuid::parse_str(transaction).is_err() {
        return Err("installerTransactionInvalid".into());
    }
    Ok(super::metadata::wide(&format!(
        "\\\\.\\pipe\\Sayweave.Installer.{transaction}"
    )))
}
pub fn server(transaction: &str, sid: &str) -> Result<Pipe> {
    unsafe {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let sddl = super::metadata::wide(&format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{sid})"));
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .map_err(|_| "installerPipeSecurity")?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        let handle = CreateNamedPipeW(
            PCWSTR(name(transaction)?.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            32768,
            32768,
            0,
            Some(&attributes),
        );
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        if handle == INVALID_HANDLE_VALUE {
            return Err("installerPipeCreate".into());
        }
        Ok(Pipe(handle))
    }
}
pub fn request(record: &MachineRecord, operation: &str) -> Result<MachineRecord> {
    let deadline = Instant::now() + Duration::from_secs(120);
    let pipe = loop {
        let handle = unsafe {
            CreateFileW(
                PCWSTR(name(&record.transaction)?.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        };
        if let Ok(handle) = handle {
            break Pipe(handle);
        }
        if Instant::now() > deadline {
            return Err("installerMaintenanceTimeout".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    unsafe {
        let mode = PIPE_NOWAIT;
        SetNamedPipeHandleState(pipe.0, Some(&mode), None, None)
            .map_err(|_| "installerPipeMode")?;
        let mut pid = 0;
        GetNamedPipeServerProcessId(pipe.0, &mut pid).map_err(|_| "installerPipeIdentity")?;
        if pid != record.server.pid || !live(&record.server) {
            return Err("installerPipeIdentity".into());
        }
    }
    let request = serde_json::to_vec(
        &serde_json::json!({"transaction":record.transaction,"operation":operation}),
    )
    .map_err(|_| "installerPipeProtocol")?;
    pipe.send(&request).map_err(|_| "installerPipeWrite")?;
    loop {
        if let Some(bytes) = pipe.receive().map_err(|_| "installerPipeRead")? {
            let reply: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| "installerPipeProtocol")?;
            if let Some(error) = reply["error"].as_str() {
                return Err(error.to_owned());
            }
            let protected = super::machine::load()?.ok_or("installerProofMissing")?;
            if reply["transaction"].as_str() != Some(record.transaction.as_str())
                || reply["phase"]
                    != serde_json::to_value(protected.phase).map_err(|_| "installerPipeProtocol")?
                || protected.target != record.target
                || protected.transaction != record.transaction
                || protected.server != record.server
            {
                return Err("installerProofChanged".into());
            }
            let _ = pipe.send(b"ack");
            return Ok(protected);
        }
        if Instant::now() > deadline || !live(&record.server) {
            return Err("installerMaintenanceTimeout".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
pub fn elevate(args: &str) -> Result<Pipe> {
    let exe = std::env::current_exe().map_err(|_| "installerExecutable")?;
    // Hold the image against write/delete while the UAC child is launched.
    use std::os::windows::fs::OpenOptionsExt;
    let _image = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ.0)
        .open(&exe)
        .map_err(|_| "installerExecutable")?;
    let file = super::metadata::wide(&exe.to_string_lossy());
    let parameters = super::metadata::wide(args);
    let mut request = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: windows::core::w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: 0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut request).map_err(|_| "installerElevationCancelled")?;
    }
    if request.hProcess.is_invalid() {
        return Err("installerElevationFailed".into());
    }
    Ok(Pipe(request.hProcess))
}
pub fn running_at(path: &std::path::Path) -> Result<bool> {
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    unsafe {
        let snapshot = Pipe(
            CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).map_err(|_| "installerProcessQuery")?,
        );
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = Process32FirstW(snapshot.0, &mut entry).is_ok();
        while found {
            if entry.th32ProcessID != std::process::id() {
                let name = String::from_utf16_lossy(&entry.szExeFile)
                    .trim_end_matches('\0')
                    .to_owned();
                if name.eq_ignore_ascii_case("sayweave.exe") {
                    let image = process_path(entry.th32ProcessID)?;
                    if super::metadata::same_path(&image.to_string_lossy(), &path.to_string_lossy())
                    {
                        return Ok(true);
                    }
                }
            }
            found = Process32NextW(snapshot.0, &mut entry).is_ok();
        }
    }
    Ok(false)
}
