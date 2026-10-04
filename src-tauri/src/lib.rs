#[cfg(feature = "desktop-acceptance")]
mod acceptance;
mod coordinator;
mod data;
mod history;
mod models;
mod speech;
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
fn read(app: &tauri::AppHandle) -> Result<Snapshot, RuntimeError> {
    app.state::<Runtime>()
        .0
        .lock()
        .map(|s| s.clone())
        .map_err(|_| RuntimeError::new("unavailable"))
}
fn publish(app: &tauri::AppHandle, snapshot: Snapshot) {
    if !read(app).is_ok_and(|current| coordinator::resize_is_current(&current, &snapshot)) {
        return;
    }
    // Reserve the larger canvas before either the event or command response reaches the renderer.
    if let Some(window) = app.get_webview_window("voice-overlay") {
        let (w, h) = overlay_size(snapshot.session.phase);
        if let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) {
            let current = size.to_logical::<f64>(scale);
            let (width, height) =
                coordinator::reserve_size((current.width, current.height), (w, h));
            let _ = window.set_size(LogicalSize::new(width, height));
        }
    }
    let _ = app.emit_to("main", "runtime:snapshot", &snapshot);
    let _ = app.emit_to("voice-overlay", "runtime:snapshot", &snapshot);
    let app_copy = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(320)).await;
        let handle = app_copy.clone();
        let _ = app_copy.run_on_main_thread(move || {
            if read(&handle)
                .is_ok_and(|current| coordinator::resize_is_current(&current, &snapshot))
                && let Some(window) = handle.get_webview_window("voice-overlay")
            {
                let (w, h) = overlay_size(snapshot.session.phase);
                let _ = window.set_size(LogicalSize::new(w, h));
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
fn window_action(
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
            let service = app.state::<models::ModelService>();
            app.state::<speech::SpeechService>().shutdown();
            service.begin_shutdown();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let service = app.state::<models::ModelService>();
                let _ = tokio::time::timeout(Duration::from_secs(5), async {
                    app.state::<speech::SpeechService>().drain().await;
                    service.finish_shutdown().await;
                })
                .await;
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
    match state
        .request(window.label(), Operation::Delete(kind, id, revision))
        .await?
    {
        Output::Deleted => Ok(()),
        _ => Err(DataError::new("unavailable")),
    }
}
fn already_running() {
    eprintln!("SAYWEAVE_ALREADY_RUNNING");
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
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Runtime(Mutex::new(Snapshot::default())))
        .invoke_handler(tauri::generate_handler![
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
            history_detail,
            history_usage,
            history_remove,
            history_retry,
            history_play,
            #[cfg(feature = "desktop-acceptance")]
            acceptance_exit
        ])
        .setup(|app| {
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
                "local.sayweave.skeleton.{}",
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
                    let service = models::ModelService::new(worker.clone(), namespace, &root)
                        .map_err(|e| e.code)?;
                    app.manage(service);
                    app.manage(speech::SpeechService::new(history::HistoryService::new(
                        worker.clone(),
                        &root,
                    )));
                    app.manage(worker);
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
                    already_running();
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
            // Acquire the per-profile lease before creating WebView2 or a second visible window.
            tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?.build()?;
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
            .title("Sayweave · 语音控件")
            .inner_size(68.0, 68.0)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .focused(false)
            .build()?;
            if let Some(monitor) = overlay.primary_monitor()? {
                let scale = monitor.scale_factor();
                let size = monitor.work_area().size.to_logical::<f64>(scale);
                let pos = monitor.work_area().position.to_logical::<f64>(scale);
                overlay.set_position(LogicalPosition::new(
                    pos.x + size.width / 2.0 - 120.0,
                    pos.y + size.height - 140.0,
                ))?;
            }
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
