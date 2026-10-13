//! Bounded local pipe. ACL restricts connections to the current user; remote clients are rejected.
use crate::wire::MAX_FRAME;
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        Storage::FileSystem::*,
        System::{Pipes::*, RemoteDesktop::ProcessIdToSessionId, Threading::*},
    },
    core::{PCWSTR, PWSTR},
};

pub struct Pipe(pub HANDLE);
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Identity {
    pub pid: u32,
    pub session: u32,
    pub created: u64,
}
pub fn same_process_alive(expected: Identity) -> windows::core::Result<bool> {
    unsafe {
        let handle = match OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            expected.pid,
        ) {
            Ok(handle) => Pipe(handle),
            Err(error)
                if error.code()
                    == windows::core::HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        if WaitForSingleObject(handle.0, 0) != WAIT_TIMEOUT {
            return Ok(false);
        }
        Ok(identity(expected.pid).is_ok_and(|now| now == expected))
    }
}
pub fn identity(pid: u32) -> windows::core::Result<Identity> {
    unsafe {
        let handle = Pipe(OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )?);
        if WaitForSingleObject(handle.0, 0) != WAIT_TIMEOUT {
            return Err(windows::core::Error::from_hresult(E_ACCESSDENIED));
        }
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        GetProcessTimes(handle.0, &mut created, &mut exited, &mut kernel, &mut user)?;
        if WaitForSingleObject(handle.0, 0) != WAIT_TIMEOUT {
            return Err(windows::core::Error::from_hresult(E_ACCESSDENIED));
        }
        Ok(Identity {
            pid,
            session: session(pid)?,
            created: ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64,
        })
    }
}
impl Drop for Pipe {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
pub fn session(pid: u32) -> windows::core::Result<u32> {
    let mut id = 0;
    unsafe {
        ProcessIdToSessionId(pid, &mut id)?;
    }
    Ok(id)
}
pub fn name(pid: u32, thread: u32) -> windows::core::Result<Vec<u16>> {
    // Direct owned fixtures coexist with the installed fixed proxy in their
    // own process. Production builds never honor this isolated endpoint.
    #[cfg(feature = "acceptance")]
    let fixture = if std::env::var_os("SAYWEAVE_BLOCK_FIXTURE").is_some() {
        ".BlockFixture"
    } else {
        ""
    };
    #[cfg(not(feature = "acceptance"))]
    let fixture = "";
    Ok(format!(
        "\\\\.\\pipe\\Sayweave.TSF.{}.{}.{}{fixture}",
        session(pid)?,
        pid,
        thread
    )
    .encode_utf16()
    .chain(Some(0))
    .collect())
}
pub fn current_user_sid() -> windows::core::Result<String> {
    process_user_sid(std::process::id())
}
pub fn process_user_sid(pid: u32) -> windows::core::Result<String> {
    unsafe {
        let process = Pipe(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)?);
        let mut token = HANDLE::default();
        OpenProcessToken(process.0, TOKEN_QUERY, &mut token)?;
        let token = Pipe(token);
        let mut needed = 0;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut needed);
        let mut storage = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(storage.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )?;
        let user = &*(storage.as_ptr() as *const TOKEN_USER);
        let mut string = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut string)?;
        let result = string.to_string();
        let _ = LocalFree(Some(HLOCAL(string.0.cast())));
        Ok(result?)
    }
}
impl Pipe {
    pub fn server() -> windows::core::Result<Self> {
        unsafe {
            let sid = current_user_sid()?;
            let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )?;
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: false.into(),
            };
            let path = name(std::process::id(), GetCurrentThreadId())?;
            let handle = CreateNamedPipeW(
                PCWSTR(path.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                MAX_FRAME as u32 + 4,
                MAX_FRAME as u32 + 4,
                0,
                Some(&attributes),
            );
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
            if handle == INVALID_HANDLE_VALUE {
                Err(windows::core::Error::from_thread())
            } else {
                Ok(Self(handle))
            }
        }
    }
    pub fn client(pid: u32, thread: u32) -> windows::core::Result<Self> {
        unsafe {
            let expected = identity(pid)?;
            let path = name(pid, thread)?;
            let pipe = Self(CreateFileW(
                PCWSTR(path.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )?);
            let mode = PIPE_NOWAIT;
            SetNamedPipeHandleState(pipe.0, Some(&mode), None, None)?;
            let mut server = 0;
            GetNamedPipeServerProcessId(pipe.0, &mut server)?;
            if server != pid
                || identity(server)? != expected
                || expected.session != session(std::process::id())?
            {
                return Err(windows::core::Error::from_hresult(E_ACCESSDENIED));
            }
            Ok(pipe)
        }
    }
    pub fn connect(&self) -> bool {
        unsafe {
            ConnectNamedPipe(self.0, None).is_ok()
                || matches!(GetLastError(), ERROR_PIPE_CONNECTED | ERROR_NO_DATA)
        }
    }
    pub fn disconnect(&self) {
        unsafe {
            let _ = DisconnectNamedPipe(self.0);
        }
    }
    pub fn client_pid(&self) -> windows::core::Result<u32> {
        let mut pid = 0;
        unsafe {
            GetNamedPipeClientProcessId(self.0, &mut pid)?;
        }
        Ok(pid)
    }
    pub fn alive(&self) -> bool {
        unsafe { PeekNamedPipe(self.0, None, 0, None, None, None).is_ok() }
    }
    pub fn receive(&self) -> windows::core::Result<Option<Vec<u8>>> {
        unsafe {
            let mut available = 0;
            let mut header = [0u8; 4];
            let mut copied = 0;
            PeekNamedPipe(
                self.0,
                Some(header.as_mut_ptr().cast()),
                4,
                Some(&mut copied),
                Some(&mut available),
                None,
            )?;
            if available < 4 {
                return Ok(None);
            }
            let length = u32::from_le_bytes(header) as usize;
            if length > MAX_FRAME {
                return Err(windows::core::Error::from_hresult(E_INVALIDARG));
            }
            if available < (length + 4) as u32 {
                return Ok(None);
            }
            let mut bytes = vec![0; length + 4];
            let mut read = 0;
            ReadFile(self.0, Some(&mut bytes), Some(&mut read), None)?;
            if read as usize != bytes.len() {
                return Err(windows::core::Error::from_hresult(E_FAIL));
            }
            Ok(Some(bytes.split_off(4)))
        }
    }
    pub fn send(&self, data: &[u8]) -> windows::core::Result<()> {
        if data.len() > MAX_FRAME {
            return Err(windows::core::Error::from_hresult(E_INVALIDARG));
        }
        let mut frame = Vec::with_capacity(data.len() + 4);
        frame.extend_from_slice(&(data.len() as u32).to_le_bytes());
        frame.extend_from_slice(data);
        let mut written = 0;
        unsafe {
            WriteFile(self.0, Some(&frame), Some(&mut written), None)?;
        }
        if written as usize == frame.len() {
            Ok(())
        } else {
            Err(windows::core::Error::from_hresult(E_FAIL))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;
    #[test]
    fn retained_process_handle_does_not_make_a_terminated_host_live() {
        let mut child = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
            .creation_flags(CREATE_NO_WINDOW.0)
            .spawn()
            .unwrap();
        let pid = child.id();
        assert!(identity(pid).is_ok());
        let held =
            unsafe { Pipe(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).unwrap()) };
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(identity(pid).is_err());
        drop(held);
    }
}
