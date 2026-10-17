#[cfg(feature = "desktop-acceptance")]
mod acceptance;
mod appearance;
mod application_exit;
mod archive;
mod build_info;
mod contracts;
mod coordinator;
mod data;
mod dictation;
mod history;
mod input_state;
mod installer;
mod language_learning;
mod language_policy;
mod lifecycle;
mod models;
mod network;
mod processing;
mod processing_commands;
mod shortcuts;
mod speech;
mod speech_projection;
mod task_stop;
mod text_service;
mod tls;
mod update;
#[cfg(feature = "desktop-acceptance")]
mod update_acceptance;
pub mod update_signature;
mod voice_window;
pub use installer::helper_entry as installer_helper_entry;
pub use text_service::helper_entry as text_service_helper_entry;
use text_service::{text_service_action, text_service_status};
mod windows_input;
use dictation::*;
use shortcuts::commands::*;
static OVERLAY_VISIBILITY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
use coordinator::{Phase, RuntimeError, Snapshot, authorized_window, overlay_size};
use data::{
    DataWorker, Operation, Output,
    model::{DataError, Input, Kind, Page, Query, Record, Status},
};
use models::commands::*;
use speech::commands::*;
use std::{sync::Mutex, time::Duration};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, LogicalPosition, LogicalSize, Manager, State, WebviewWindow};
struct Runtime(Mutex<Snapshot>);
#[cfg(feature = "desktop-acceptance")]
pub fn input_acceptance(fixture: &std::path::Path) -> Result<(), String> {
    windows_input::native_checks(fixture)
}
#[cfg(feature = "desktop-acceptance")]
pub fn composition_acceptance(fixture: &std::path::Path) -> Result<(), String> {
    windows_input::composition_checks(fixture)
}
#[cfg(feature = "desktop-acceptance")]
pub fn observation_acceptance(process: u32) -> Result<(), String> {
    windows_input::observation_checks(process)
}
#[cfg(feature = "desktop-acceptance")]
pub fn endpoint_acceptance(root: u32, editor: u32) -> Result<(), String> {
    windows_input::endpoint_checks(root, editor)
}
#[cfg(feature = "desktop-acceptance")]
#[tauri::command]
fn acceptance_exit(app: tauri::AppHandle, window: WebviewWindow) -> Result<(), RuntimeError> {
    if window.label() != "main"
        || !std::env::var("SAYWEAVE_DATA_ACCEPTANCE_ID")
            .is_ok_and(|id| uuid::Uuid::parse_str(&id).is_ok())
    {
        return Err(RuntimeError::new("forbidden"));
    }
    tray_action(&app, "exit")
}
#[cfg(feature = "desktop-acceptance")]
#[tauri::command]
fn acceptance_main_size(
    window: WebviewWindow,
    minimum: bool,
    zoom: Option<f64>,
) -> Result<f64, RuntimeError> {
    let test_build = crate::build_info::current().channel == "test"
        && env!("SAYWEAVE_BUILD_IDENTIFIER").starts_with("local.sayweave.acceptance.");
    if window.label() != "main"
        || !(test_build
            || std::env::var("SAYWEAVE_DATA_ACCEPTANCE_ID")
                .is_ok_and(|id| uuid::Uuid::parse_str(&id).is_ok()))
    {
        return Err(RuntimeError::new("forbidden"));
    }
    let size = if minimum {
        (960.0, 640.0)
    } else {
        (1120.0, 760.0)
    };
    window
        .set_size(LogicalSize::new(size.0, size.1))
        .map_err(|_| RuntimeError::new("unavailable"))?;
    let zoom = zoom.unwrap_or(1.0);
    if ![1.0, 1.5].contains(&zoom) {
        return Err(RuntimeError::new("invalid"));
    }
    window
        .set_zoom(zoom)
        .map_err(|_| RuntimeError::new("unavailable"))?;
    window
        .scale_factor()
        .map_err(|_| RuntimeError::new("unavailable"))
}
fn read(app: &tauri::AppHandle) -> Result<Snapshot, RuntimeError> {
    app.state::<Runtime>()
        .0
        .lock()
        .map(|s| s.clone())
        .map_err(|_| RuntimeError::new("unavailable"))
}
#[derive(Clone, PartialEq)]
struct VoiceLayout {
    view: Option<(Option<String>, Option<String>, Presentation)>,
    size: (f64, f64),
    instance: Option<u64>,
}
fn voice_view(
    snapshot: dictation::Snapshot,
) -> Option<(Option<String>, Option<String>, Presentation)> {
    (snapshot.source == "external" && snapshot.presentation != Presentation::Idle).then_some((
        snapshot.task_id,
        snapshot.presentation_id,
        snapshot.presentation,
    ))
}
fn voice_layout(app: &tauri::AppHandle, phase: Phase) -> VoiceLayout {
    let view = app
        .try_state::<dictation::DictationCoordinator>()
        .and_then(|c| c.snapshot().ok())
        .and_then(voice_view);
    let size = if let Some((_, _, presentation)) = &view {
        match presentation {
            Presentation::Result => (384.0, 232.0),
            Presentation::Error => (384.0, 112.0),
            _ => (160.0, 88.0),
        }
    } else {
        overlay_size(phase)
    };
    let instance = app
        .get_webview_window("voice-overlay")
        .and_then(|w| voice_window::instance(&w));
    VoiceLayout {
        view,
        size,
        instance,
    }
}
fn refresh_voice(app: &tauri::AppHandle) {
    if let Ok(snapshot) = read(app) {
        publish(app, snapshot);
    }
}
#[cfg(test)]
mod voice_layout_tests {
    use super::*;
    #[test]
    fn content_keeps_layout_identity_but_new_task_or_presentation_invalidates_delay() {
        let mut snapshot = dictation::Snapshot {
            source: "external".into(),
            task_id: Some("first".into()),
            presentation_id: Some("view".into()),
            presentation: Presentation::Recording,
            ..Default::default()
        };
        let first = voice_view(snapshot.clone());
        snapshot.sequence += 1;
        snapshot.text = "new caption".into();
        assert_eq!(voice_view(snapshot.clone()), first);
        snapshot.presentation = Presentation::Recognizing;
        assert_ne!(voice_view(snapshot.clone()), first);
        snapshot.presentation = Presentation::Recording;
        snapshot.task_id = Some("next".into());
        assert_ne!(voice_view(snapshot.clone()), first);
        snapshot.task_id = Some("first".into());
        snapshot.presentation_id = Some("new-view".into());
        assert_ne!(voice_view(snapshot), first);
    }
}
fn publish(app: &tauri::AppHandle, snapshot: Snapshot) {
    if !read(app).is_ok_and(|current| coordinator::resize_is_current(&current, &snapshot)) {
        return;
    }
    let layout = voice_layout(app, snapshot.session.phase);
    // Validate again on the owning thread; a queued layout cannot affect a newer session.
    let reserve_app = app.clone();
    let reserve_snapshot = snapshot.clone();
    let reserve_layout = layout.clone();
    let _ = app.run_on_main_thread(move || {
        if !read(&reserve_app)
            .is_ok_and(|current| coordinator::resize_is_current(&current, &reserve_snapshot))
            || voice_layout(&reserve_app, reserve_snapshot.session.phase) != reserve_layout
        {
            return;
        }
        if let Some(window) = reserve_app.get_webview_window("voice-overlay") {
            let (w, h) = reserve_layout.size;
            if let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) {
                let current = size.to_logical::<f64>(scale);
                let (width, height) =
                    coordinator::reserve_size((current.width, current.height), (w, h));
                voice_window::layout(
                    &window,
                    width,
                    height,
                    reserve_layout.view.as_ref().and_then(|v| v.0.as_deref()),
                );
            }
        }
    });
    let _ = app.emit_to("main", "runtime:snapshot", &snapshot);
    let _ = app.emit_to("voice-overlay", "runtime:snapshot", &snapshot);
    let app_copy = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(320)).await;
        let handle = app_copy.clone();
        let _ = app_copy.run_on_main_thread(move || {
            if read(&handle)
                .is_ok_and(|current| coordinator::resize_is_current(&current, &snapshot))
                && voice_layout(&handle, snapshot.session.phase) == layout
                && let Some(window) = handle.get_webview_window("voice-overlay")
            {
                let (w, h) = layout.size;
                voice_window::layout(
                    &window,
                    w,
                    h,
                    layout.view.as_ref().and_then(|v| v.0.as_deref()),
                );
            }
        });
    });
}
#[tauri::command]
fn runtime_snapshot(state: State<'_, Runtime>) -> Result<Snapshot, RuntimeError> {
    state
        .0
        .lock()
        .map(|s| s.clone())
        .map_err(|_| RuntimeError::new("unavailable"))
}
fn delayed(app: tauri::AppHandle, id: u64, expected: Phase, phase: Phase, ms: u64) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        let update = {
            let runtime = app.state::<Runtime>();
            let Ok(mut s) = runtime.0.lock() else {
                return;
            };
            if s.settle(id, expected, phase) {
                Some(s.clone())
            } else {
                None
            }
        };
        if let Some(s) = update {
            publish(&app, s);
        }
    });
}
#[tauri::command]
fn runtime_demo(
    app: tauri::AppHandle,
    window: WebviewWindow,
    action: String,
    session_id: u64,
) -> Result<Snapshot, RuntimeError> {
    if (!cfg!(debug_assertions) && !cfg!(feature = "desktop-acceptance"))
        || app.state::<speech::SpeechService>().is_active()
        || app.state::<dictation::DictationCoordinator>().busy()
    {
        return Err(RuntimeError::new("forbidden"));
    }
    if !["main", "voice-overlay"].contains(&window.label()) {
        return Err(RuntimeError::new("forbidden"));
    }
    let snapshot = {
        let runtime = app.state::<Runtime>();
        let mut s = runtime
            .0
            .lock()
            .map_err(|_| RuntimeError::new("unavailable"))?;
        s.demo(&action, session_id)?;
        s.clone()
    };
    publish(&app, snapshot.clone());
    match action.as_str() {
        "start" => delayed(
            app,
            snapshot.session.id,
            Phase::Connecting,
            Phase::Recording,
            650,
        ),
        "finish" => {
            delayed(
                app.clone(),
                snapshot.session.id,
                Phase::Processing,
                Phase::Completed,
                900,
            );
            delayed(
                app,
                snapshot.session.id,
                Phase::Completed,
                Phase::Idle,
                1900,
            );
        }
        _ => (),
    }
    Ok(snapshot)
}
fn overlay_visibility(window: &WebviewWindow, visible: bool) -> tauri::Result<()> {
    let task = window
        .app_handle()
        .try_state::<DictationCoordinator>()
        .and_then(|c| c.snapshot().ok())
        .filter(|s| s.source == "external")
        .and_then(|s| s.task_id);
    voice_window::visibility(window, visible, task)
}
fn window_action_internal(
    app: &tauri::AppHandle,
    surface: &str,
    action: &str,
) -> Result<Snapshot, RuntimeError> {
    let label = match surface {
        "main" => "main",
        "overlay" => "voice-overlay",
        _ => return Err(RuntimeError::new("invalidAction")),
    };
    let window = app
        .get_webview_window(label)
        .ok_or(RuntimeError::new("unavailable"))?;
    let result = match action {
        "show" if surface == "overlay" => overlay_visibility(&window, true),
        "hide" if surface == "overlay" => overlay_visibility(&window, false),
        "show" => window.show().and_then(|_| {
            if surface == "main" {
                window.unminimize().and_then(|_| window.set_focus())
            } else {
                Ok(())
            }
        }),
        "hide" => window.hide(),
        "minimize" if surface == "main" => window.minimize(),
        "toggleMaximize" if surface == "main" => {
            if window.is_maximized().unwrap_or(false) {
                window.unmaximize()
            } else {
                window.maximize()
            }
        }
        _ => return Err(RuntimeError::new("invalidAction")),
    };
    result.map_err(|_| RuntimeError::new("unavailable"))?;
    let snapshot = {
        let runtime = app.state::<Runtime>();
        let mut s = runtime
            .0
            .lock()
            .map_err(|_| RuntimeError::new("unavailable"))?;
        if action == "show" || action == "hide" {
            s.window(surface, action == "show")?;
        }
        s.clone()
    };
    publish(app, snapshot.clone());
    Ok(snapshot)
}
fn window_action(
    app: &tauri::AppHandle,
    surface: &str,
    action: &str,
) -> Result<Snapshot, RuntimeError> {
    if surface == "overlay" && ["show", "hide"].contains(&action) {
        OVERLAY_VISIBILITY.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
    window_action_internal(app, surface, action)
}
fn tray_action(app: &tauri::AppHandle, id: &str) -> Result<(), RuntimeError> {
    match id {
        "open" => window_action(app, "main", "show").map(|_| ()),
        "voice" => window_action(app, "overlay", "show").map(|_| ()),
        "hide" => window_action(app, "overlay", "hide").map(|_| ()),
        "exit" => {
            let runtime = app.state::<Runtime>();
            let mut state = runtime
                .0
                .lock()
                .map_err(|_| RuntimeError::new("unavailable"))?;
            state.shutdown();
            println!(
                "SAYWEAVE_SHUTDOWN revision={} phase={:?}",
                state.revision, state.session.phase
            );
            drop(state);
            let lifecycle = app.state::<lifecycle::Lifecycle>();
            let _ = lifecycle.fence(|| false);
            let watermark = archive::watermark();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let stopped = tokio::time::timeout(Duration::from_secs(5), async {
                    app.state::<lifecycle::Lifecycle>()
                        .drain()
                        .await
                        .map_err(str::to_owned)?;
                    application_exit::stop(&app, watermark).await
                })
                .await;
                if !matches!(stopped, Ok(Ok(()))) {
                    log::error!(target:"sayweave","operation=shutdown result=failed");
                }
                log::logger().flush();
                app.exit(0);
            });
            Ok(())
        }
        _ => Err(RuntimeError::new("invalidAction")),
    }
}
#[tauri::command]
fn runtime_window(
    app: tauri::AppHandle,
    window: WebviewWindow,
    surface: String,
    action: String,
) -> Result<Snapshot, RuntimeError> {
    if !authorized_window(window.label(), &surface, &action) {
        return Err(RuntimeError::new("forbidden"));
    }
    window_action(&app, &surface, &action)
}
#[tauri::command]
async fn dictionary_status(
    window: WebviewWindow,
    state: State<'_, DataWorker>,
    retry: bool,
) -> Result<Status, DataError> {
    data::model::authorize(window.label())?;
    if retry {
        if let Output::Status(status) = state.request(window.label(), Operation::Retry).await? {
            return Ok(status);
        }
        return Err(DataError::new("unavailable"));
    }
    state.status()
}
#[tauri::command]
async fn dictionary_list(
    window: WebviewWindow,
    state: State<'_, DataWorker>,
    query: Query,
) -> Result<Page, DataError> {
    match state
        .request(window.label(), Operation::List(query))
        .await?
    {
        Output::Page(page) => Ok(page),
        _ => Err(DataError::new("unavailable")),
    }
}
#[tauri::command]
async fn dictionary_get(
    window: WebviewWindow,
    state: State<'_, DataWorker>,
    kind: Kind,
    id: String,
) -> Result<Record, DataError> {
    match state
        .request(window.label(), Operation::Get(kind, id))
        .await?
    {
        Output::Record(record) => Ok(record),
        _ => Err(DataError::new("unavailable")),
    }
}
#[tauri::command]
async fn dictionary_create(
    window: WebviewWindow,
    state: State<'_, DataWorker>,
    input: Input,
) -> Result<Record, DataError> {
    let _admission = lifecycle::mutation(&window)?;
    match state
        .request(window.label(), Operation::Create(input))
        .await?
    {
        Output::Record(record) => Ok(record),
        _ => Err(DataError::new("unavailable")),
    }
}
#[tauri::command]
async fn dictionary_update(
    window: WebviewWindow,
    state: State<'_, DataWorker>,
    input: Input,
    id: String,
    revision: i64,
) -> Result<Record, DataError> {
    let _admission = lifecycle::mutation(&window)?;
    match state
        .request(window.label(), Operation::Update(input, id, revision))
        .await?
    {
        Output::Record(record) => Ok(record),
        _ => Err(DataError::new("unavailable")),
    }
}
#[tauri::command]
async fn dictionary_delete(
    window: WebviewWindow,
    state: State<'_, DataWorker>,
    kind: Kind,
    id: String,
    revision: i64,
) -> Result<(), DataError> {
    let _admission = lifecycle::mutation(&window)?;
    match state
        .request(window.label(), Operation::Delete(kind, id, revision))
        .await?
    {
        Output::Deleted => Ok(()),
        _ => Err(DataError::new("unavailable")),
    }
}
fn already_running(launch: installer::login_startup::LaunchReason) {
    eprintln!("SAYWEAVE_ALREADY_RUNNING");
    if !launch.main_visible() {
        return;
    }
    #[cfg(feature = "desktop-acceptance")]
    if std::env::var("SAYWEAVE_SECOND_INSTANCE_ACCEPTANCE").as_deref() == Ok("1") {
        return;
    }
    #[cfg(target_os = "windows")]
    {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn MessageBoxW(
                hwnd: *mut std::ffi::c_void,
                text: *const u16,
                caption: *const u16,
                flags: u32,
            ) -> i32;
        }
        let text: Vec<u16> = "Sayweave 已在运行，请从托盘打开。\0"
            .encode_utf16()
            .collect();
        let title: Vec<u16> = "Sayweave\0".encode_utf16().collect();
        // Null parent, null-terminated owned UTF-16 buffers, informational modal.
        unsafe {
            MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x40);
        }
    }
}
pub fn run() {
    let launch = installer::login_startup::LaunchReason::current();
    tls::initialize();
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if let Some(service) = app.try_state::<shortcuts::ShortcutService>() {
                        service.plugin_event(event.state);
                    }
                })
                .build(),
        )
        .manage(dictation::DictationCoordinator::default())
        .manage(windows_input::WindowsInputService::new())
        .manage(Runtime(Mutex::new(Snapshot::with_main_visibility(launch.main_visible()))))
        .manage(lifecycle::Lifecycle::default())
        .on_page_load(|webview, payload| {
            if webview.label() == "main"
                && payload.event() == tauri::webview::PageLoadEvent::Finished
            {
                let build = build_info::current();
                let at_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis();
                log::info!(target: "sayweave", "operation=startup phase=main_page_ready pid={} at_ms={} version={} commit={} channel={}", std::process::id(), at_ms, build.version, build.commit, build.channel);
            }
            if webview.label() == "main"
                && payload.event() == tauri::webview::PageLoadEvent::Started
                && let Some(service) = webview
                    .app_handle()
                    .try_state::<shortcuts::ShortcutService>()
            {
                let _ = service.capture(webview.app_handle(), false);
            }
        })
        .invoke_handler(tauri::generate_handler![
            appearance::appearance_read,
            appearance::appearance_save,
            update::update_snapshot,
            update::update_check,
            update::update_cancel,
            update::update_install,
            update::update_restart,
            #[cfg(feature = "desktop-acceptance")]
            update_acceptance::acceptance_update_seed,
            #[cfg(feature = "desktop-acceptance")]
            update_acceptance::acceptance_update_data,
            #[cfg(feature = "desktop-acceptance")]
            update_acceptance::acceptance_update_exit,
            #[cfg(feature = "desktop-acceptance")]
            update_acceptance::acceptance_update_hold,
            processing_commands::processing_status,
            processing_commands::processing_settings,
            processing_commands::processing_save,
            shortcut_status,
            shortcut_save,
            shortcut_capture,
            text_service_status,
            text_service_action,
            dictation_snapshot,
            dictation_start,
            dictation_control,
            dictation_copy,
            dictation_dismiss,
            dictation_settings,
            language_learning::language_settings,
            language_learning::language_overview,
            language_learning::language_profile,
            language_learning::language_save,
            runtime_snapshot,
            runtime_demo,
            runtime_window,
            dictionary_status,
            dictionary_list,
            dictionary_get,
            dictionary_create,
            dictionary_update,
            dictionary_delete,
            model_status,
            model_overview,
            model_save,
            model_remove,
            model_default,
            model_fetch,
            model_test,
            model_cancel,
            model_help,
            speech_snapshot,
            speech_devices,
            speech_start,
            speech_retry_current,
            speech_control,
            speech_preferences,
            speech_copy,
            history_status,
            history_list,
            history_browse,
            history_detail,
            history_delivery,
            history_usage,
            history_remove,
            history_retry,
            history_play,
            #[cfg(feature = "desktop-acceptance")]
            acceptance_exit,
            #[cfg(feature = "desktop-acceptance")]
            acceptance_main_size
        ])
        .setup(move |app| {
            eprintln!("SAYWEAVE_STARTUP phase=setup pid={}", std::process::id());
            if app.config().identifier != env!("SAYWEAVE_BUILD_IDENTIFIER") {
                return Err("compiled application identity mismatch".into());
            }
            let base = app.path().app_data_dir()?;
            #[cfg(feature = "desktop-acceptance")]
            let base = match std::env::var("SAYWEAVE_DATA_ACCEPTANCE_ID") {
                Ok(id) if uuid::Uuid::parse_str(&id).is_ok() => {
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../.local/data-acceptance")
                        .join(id)
                }
                _ => base,
            };
            let root = data::profile_root(&base, cfg!(debug_assertions));
            #[cfg(feature = "desktop-acceptance")]
            println!("SAYWEAVE_DATA_DIRECTORY {}", root.display());
            let namespace = format!(
                "{}.{}",
                env!("SAYWEAVE_BUILD_IDENTIFIER"),
                if cfg!(debug_assertions) {
                    "dev"
                } else {
                    "release"
                }
            );
            #[cfg(feature = "desktop-acceptance")]
            let namespace = match std::env::var("SAYWEAVE_DATA_ACCEPTANCE_ID") {
                Ok(id) if uuid::Uuid::parse_str(&id).is_ok() => {
                    format!("local.sayweave.skeleton.acceptance.{id}")
                }
                _ => namespace,
            };
            match DataWorker::start(root.clone()) {
                Ok(worker) => {
                    let requests = std::sync::Arc::new(network::RequestCoordinator::default());
                    app.manage(
                        update::UpdateService::new(&root, requests.clone())
                            .map_err(|e| format!("update setup: {e:?}"))?,
                    );
                    let service =
                        models::ModelService::new(worker.clone(), namespace, &root, requests)
                            .map_err(|e| e.code)?;
                    app.manage(service);
                    app.manage(speech::SpeechService::new(history::HistoryService::new(
                        worker.clone(),
                        &root,
                    )));
                    app.manage(shortcuts::ShortcutService::new(worker.clone()));
                    app.manage(worker);
                    app.manage(language_learning::LanguageLearning::new(
                        app.handle().clone(),
                    ));
                    update::schedule(app.handle().clone());
                    let handle = app.handle().clone();
                    let mut transcripts = handle
                        .state::<speech::SpeechService>()
                        .subscribe_transcripts();
                    tauri::async_runtime::spawn(async move {
                        while transcripts.changed().await.is_ok() {
                            let transcript = transcripts.borrow_and_update().clone();
                            handle
                                .state::<dictation::DictationCoordinator>()
                                .observe_transcript(&transcript);
                        }
                    });
                    speech_projection::attach(app.handle())
                        .map_err(|e| std::io::Error::other(e.code))?;
                    let handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        // Recover before accepting speech; a data fault does not prevent the window opening.
                        loop {
                            let service = handle.state::<speech::SpeechService>();
                            if service.is_stopping() {
                                break;
                            }
                            let result = service.maintain().await;
                            tokio::time::sleep(std::time::Duration::from_secs(if result.is_ok() {
                                86400
                            } else {
                                60
                            }))
                            .await;
                        }
                    });
                }
                Err(error) if error.code == "alreadyRunning" => {
                    already_running(launch);
                    app.handle().exit(0);
                    return Ok(());
                }
                Err(error) => {
                    return Err(format!("data worker initialization: {}", error.code).into());
                }
            }
            // Only our structured, whitelisted diagnostics reach disk; no external HTTP/debug targets.
            let logger = tauri_plugin_log::Builder::new()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stderr),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Folder {
                        path: root.join("logs"),
                        file_name: Some("sayweave".into()),
                    }),
                ])
                .max_file_size(2 * 1024 * 1024)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(3))
                .level(log::LevelFilter::Info)
                .filter(|metadata| metadata.target() == "sayweave")
                .build();
            if app.handle().plugin(logger).is_err() {
                eprintln!("SAYWEAVE_LOG unavailable");
                // Disk failure must not silently discard subsequent safe diagnostics.
                let fallback = tauri_plugin_log::Builder::new()
                    .targets([tauri_plugin_log::Target::new(
                        tauri_plugin_log::TargetKind::Stderr,
                    )])
                    .level(log::LevelFilter::Info)
                    .filter(|metadata| metadata.target() == "sayweave")
                    .build();
                if app.handle().plugin(fallback).is_err() {
                    eprintln!("SAYWEAVE_LOG stderr_unavailable");
                }
            }
            let appearance_service=appearance::AppearanceService::new(&root);
            let appearance_initial=appearance_service.startup();
            let appearance_bootstrap=appearance::bootstrap(&appearance_initial);
            app.manage(appearance_service);
            // Acquire the per-profile lease before creating WebView2 or a second visible window.
            tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                .data_directory(root.join("webview2"))
                .background_color(appearance::background(&appearance_initial.theme_id))
                .initialization_script(&appearance_bootstrap)
                .visible(launch.main_visible())
                .focused(launch.main_visible())
                .build()?;
            let build = build_info::current();
            log::info!(target: "sayweave", "operation=startup phase=main_window_created pid={} version={} commit={} channel={} reason={launch:?} main_visible={}", std::process::id(), build.version, build.commit, build.channel, launch.main_visible());
            if let Some(main) = app.get_webview_window("main")
                && let Some(monitor) = main.primary_monitor()?
            {
                let scale = monitor.scale_factor();
                let area = monitor.work_area();
                let size = area.size.to_logical::<f64>(scale);
                let origin = area.position.to_logical::<f64>(scale);
                let width = 1120.0_f64.min((size.width * 0.9).max(960.0));
                let height = 760.0_f64.min((size.height * 0.9).max(640.0));
                main.set_size(LogicalSize::new(width, height))?;
                main.set_position(LogicalPosition::new(
                    origin.x + (size.width - width) / 2.0,
                    origin.y + (size.height - height) / 2.0,
                ))?;
                println!(
                    "SAYWEAVE_DISPLAY scale={scale} work={}x{} initial={width}x{height}",
                    size.width, size.height
                );
            }
            let overlay = tauri::WebviewWindowBuilder::new(
                app,
                "voice-overlay",
                tauri::WebviewUrl::App("index.html?surface=overlay".into()),
            )
            .data_directory(root.join("webview2"))
            .initialization_script(&appearance_bootstrap)
            .title("Sayweave · 语音控件")
            .inner_size(160.0, 88.0)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .focused(false)
            .focusable(false)
            .build()?;
            app.state::<windows_input::WindowsInputService>()
                .set_owner(overlay.hwnd()?.0 as usize);
            voice_window::install(overlay.hwnd()?.0 as usize)?;
            let startup = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = startup
                    .state::<shortcuts::ShortcutService>()
                    .initialize(&startup)
                    .await
                {
                    log::warn!(target:"sayweave","operation=shortcut_initialize error={}",e.code);
                }
            });
            voice_window::reposition(&overlay, None);
            let open = MenuItem::with_id(app, "open", "打开 Sayweave", true, None::<&str>)?;
            let voice = MenuItem::with_id(app, "voice", "显示语音控件", true, None::<&str>)?;
            let hide = MenuItem::with_id(app, "hide", "隐藏语音控件", true, None::<&str>)?;
            let exit = MenuItem::with_id(app, "exit", "退出 Sayweave", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &voice, &hide, &exit])?;
            TrayIconBuilder::with_id("sayweave-tray")
                .icon(
                    app.default_window_icon()
                        .expect("bundled application icon")
                        .clone(),
                )
                .tooltip("Sayweave")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| {
                    let _ = tray_action(app, event.id.as_ref());
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        let _ = window_action(tray.app_handle(), "main", "show");
                    }
                })
                .build(app)?;
            println!("SAYWEAVE_NATIVE_READY");
            #[cfg(feature = "desktop-acceptance")]
            if std::env::var("SAYWEAVE_LANGUAGE_ACTOR_ACCEPTANCE").as_deref() == Ok("1") {
                models::language_actor_acceptance::start(app.handle().clone());
            }
            #[cfg(feature = "desktop-acceptance")]
            if std::env::var("SAYWEAVE_DESKTOP_ACCEPTANCE").as_deref() == Ok("1") {
                let ids = menu
                    .items()?
                    .iter()
                    .map(|item| item.id().as_ref().to_owned())
                    .collect();
                acceptance::start(app.handle().clone(), ids);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "voice-overlay"
                && matches!(event, tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Moved(_) | tauri::WindowEvent::ScaleFactorChanged { .. })
                && let Some(overlay) = window.app_handle().get_webview_window("voice-overlay")
            {
                voice_window::reposition(&overlay, None);
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window_action(
                    window.app_handle(),
                    if window.label() == "main" {
                        "main"
                    } else {
                        "overlay"
                    },
                    "hide",
                );
            }
        })
        .run(tauri::generate_context!())
        .expect("Sayweave desktop runtime failed");
}
