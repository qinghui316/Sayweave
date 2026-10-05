//! Main-window component lifecycle; system registration is an explicit fixed helper action.
use crate::history::types::Error;
use serde::{Deserialize, Serialize};
use tauri::{State, WebviewWindow};
const PAYLOAD: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/text-service.dll"));
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    installed: bool,
    enabled: bool,
    update_available: bool,
    payload_available: bool,
    cleanup_pending: bool,
    error: Option<String>,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Enable,
    Disable,
    Install,
    Remove,
}
#[cfg(windows)]
fn apartment<T>(operation: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    use windows::Win32::System::Com::*;
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|_| Error::new("compositionApartment"))?;
    }
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    let _apartment = Apartment;
    operation()
}
#[cfg(not(windows))]
fn apartment<T>(operation: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    operation()
}
#[cfg(windows)]
fn status() -> Result<Status, Error> {
    use sayweave_text_service::{deployment, registration};
    let installed = registration::owned_module()
        .map_err(|_| Error::new("componentRegistration"))?
        .is_some();
    let matches = deployment::matches_payload(PAYLOAD);
    let cleanup = deployment::cleanup_pending();
    let enabled = if installed && matches.is_ok() {
        (|| unsafe {
            let profiles =
                registration::profiles().map_err(|_| Error::new("componentQueryFailed"))?;
            let language = profiles
                .GetCurrentLanguage()
                .map_err(|_| Error::new("componentQueryFailed"))?;
            profiles
                .IsEnabledLanguageProfile(&registration::CLASS, language, &registration::PROFILE)
                .map(|v| v.as_bool())
                .map_err(|_| Error::new("componentQueryFailed"))
        })()
    } else {
        Ok(false)
    };
    Ok(Status {
        installed,
        enabled: enabled.as_ref().copied().unwrap_or(false),
        update_available: installed && !PAYLOAD.is_empty() && !matches!(matches, Ok(Some(true))),
        payload_available: !PAYLOAD.is_empty(),
        cleanup_pending: cleanup.as_ref().copied().unwrap_or(false),
        error: if matches.is_err() || cleanup.is_err() {
            Some("componentRegistration".into())
        } else {
            enabled.err().map(|e| e.code)
        },
    })
}
/// Called in the host's COM apartment, using exactly the status shown by settings.
#[cfg(windows)]
pub(crate) fn require_ready() -> Result<(), Error> {
    let value = status()?;
    if let Some(code) = value.error {
        return Err(Error::new(&code));
    }
    if !value.installed {
        return Err(Error::new("componentMissing"));
    }
    if value.update_available {
        return Err(Error::new("componentVersionMismatch"));
    }
    if !value.enabled {
        return Err(Error::new("componentNotEnabled"));
    }
    Ok(())
}
#[cfg(not(windows))]
fn status() -> Result<Status, Error> {
    Ok(Status {
        installed: false,
        enabled: false,
        update_available: false,
        payload_available: false,
        cleanup_pending: false,
        error: Some("platform".into()),
    })
}
#[tauri::command]
pub async fn text_service_status(window: WebviewWindow) -> Result<Status, Error> {
    crate::data::model::authorize(window.label())?;
    tauri::async_runtime::spawn_blocking(|| apartment(status))
        .await
        .map_err(|_| Error::new("componentUnavailable"))?
}
#[tauri::command]
pub async fn text_service_action(
    window: WebviewWindow,
    coordinator: State<'_, crate::dictation::DictationCoordinator>,
    action: Action,
) -> Result<Status, Error> {
    crate::data::model::authorize(window.label())?;
    let maintenance = coordinator.component_maintenance()?;
    tauri::async_runtime::spawn_blocking(move || apartment(|| operate(action, maintenance)))
        .await
        .map_err(|_| Error::new("componentUnavailable"))??;
    let value = text_service_status(window).await?;
    log::info!(target:"sayweave","operation=text_service_maintenance installed={} enabled={} update_available={}",value.installed,value.enabled,value.update_available);
    Ok(value)
}
#[cfg(windows)]
fn operate(
    action: Action,
    maintenance: crate::dictation::ComponentMaintenance,
) -> Result<(), Error> {
    use sayweave_text_service::{client, registration};
    let maintenance = if matches!(action, Action::Install | Action::Remove) {
        // Registration is machine-wide. Enablement belongs to this ordinary initiating
        // user's process, not to the elevated helper or a diagnostic launch environment.
        let maintenance = elevate(action, maintenance)?;
        if matches!(action, Action::Remove) {
            return Ok(());
        }
        maintenance
    } else {
        maintenance
    };
    let _maintenance = maintenance;
    let _lease =
        registration::host_capability(std::process::id(), &uuid::Uuid::new_v4().to_string())
            .map_err(|_| Error::new("busy"))?;
    if client::speech_activation()
        .map_err(|_| Error::new("componentQueryFailed"))?
        .0
        == client::SpeechActivation::Other
    {
        return Err(Error::new("busy"));
    }
    match action {
        Action::Remove => unreachable!(),
        Action::Install | Action::Enable | Action::Disable => {
            if !status()?.installed {
                return Err(Error::new("componentMissing"));
            }
            let p = registration::profiles().map_err(|_| Error::new("componentUnavailable"))?;
            unsafe {
                let lang = p
                    .GetCurrentLanguage()
                    .map_err(|_| Error::new("componentUnavailable"))?;
                p.EnableLanguageProfile(
                    &registration::CLASS,
                    lang,
                    &registration::PROFILE,
                    !matches!(action, Action::Disable),
                )
                .map_err(|_| Error::new("componentEnableFailed"))?;
            }
            let enabled = status()?;
            if let Some(code) = enabled.error {
                return Err(Error::new(&code));
            }
            if enabled.enabled != !matches!(action, Action::Disable) {
                return Err(Error::new("componentEnableFailed"));
            }
            Ok(())
        }
    }
}
#[cfg(not(windows))]
fn operate(
    _action: Action,
    _maintenance: crate::dictation::ComponentMaintenance,
) -> Result<(), Error> {
    Err(Error::new("platform"))
}
#[cfg(windows)]
fn elevate(
    action: Action,
    maintenance: crate::dictation::ComponentMaintenance,
) -> Result<crate::dictation::ComponentMaintenance, Error> {
    use windows::{
        Win32::{Foundation::*, System::Threading::*, UI::Shell::*},
        core::{PCWSTR, w},
    };
    if matches!(action, Action::Install) && PAYLOAD.is_empty() {
        return Err(Error::new("componentPayloadMissing"));
    }
    let exe = std::env::current_exe().map_err(|_| Error::new("componentUnavailable"))?;
    let sid = sayweave_text_service::transport::current_user_sid()
        .map_err(|_| Error::new("componentUnavailable"))?;
    let args = format!(
        "--sayweave-text-service={} --initiator={sid}",
        if matches!(action, Action::Install) {
            "install"
        } else {
            "remove"
        }
    );
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let file = wide(&exe.to_string_lossy());
    let parameters = wide(&args);
    let mut request = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(parameters.as_ptr()),
        nShow: 0,
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut request).map_err(|e| {
            Error::new(if e.code() == ERROR_CANCELLED.to_hresult() {
                "componentInstallCancelled"
            } else {
                "componentInstallFailed"
            })
        })?;
        struct Process(isize);
        impl Drop for Process {
            fn drop(&mut self) {
                unsafe {
                    let _ = CloseHandle(HANDLE(self.0 as *mut _));
                }
            }
        }
        let process = Process(request.hProcess.0 as isize);
        if request.hProcess.is_invalid() {
            return Err(Error::new("componentInstallFailed"));
        }
        if WaitForSingleObject(HANDLE(process.0 as *mut _), 120000) != WAIT_OBJECT_0 {
            // A timed-out caller must not release task admission while registration may continue.
            tauri::async_runtime::spawn_blocking(move || {
                let _maintenance = maintenance;
                let _process = process;
                WaitForSingleObject(HANDLE(_process.0 as *mut _), INFINITE);
            });
            return Err(Error::new("componentInstallPending"));
        }
        let mut exit = 1;
        GetExitCodeProcess(HANDLE(process.0 as *mut _), &mut exit)
            .map_err(|_| Error::new("componentInstallFailed"))?;
        if exit != 0 {
            return Err(Error::new("componentInstallFailed"));
        }
        Ok(maintenance)
    }
}
/// The fixed elevated entry runs before Tauri/window/data startup. No general install paths.
pub fn helper_entry() -> Option<i32> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if !args
        .first()
        .is_some_and(|a| a.starts_with("--sayweave-text-service="))
    {
        return None;
    }
    #[cfg(windows)]
    {
        if args.len() != 2 {
            return Some(2);
        }
        let Some(sid) = args[1].strip_prefix("--initiator=") else {
            return Some(2);
        };
        let action = args[0].as_str();
        let result = apartment(|| {
            let _lease = sayweave_text_service::registration::host_capability(
                std::process::id(),
                &uuid::Uuid::new_v4().to_string(),
            )
            .map_err(|_| Error::new("busy"))?;
            if sayweave_text_service::client::speech_activation()
                .map_err(|_| Error::new("componentQueryFailed"))?
                .0
                == sayweave_text_service::client::SpeechActivation::Other
            {
                return Err(Error::new("busy"));
            }
            let r = match action {
                "--sayweave-text-service=install" => {
                    sayweave_text_service::deployment::install(PAYLOAD, sid)
                }
                "--sayweave-text-service=remove" => sayweave_text_service::deployment::remove(sid),
                _ => return Err(Error::new("invalidAction")),
            };
            r.map_err(|_| Error::new("componentInstallFailed"))
        });
        Some(if result.is_ok() { 0 } else { 1 })
    }
    #[cfg(not(windows))]
    Some(2)
}
