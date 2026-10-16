//! Opt-in acceptance build: exercises real Tauri windows and the tray's shared handler.
//! It does not inject desktop input or claim to click the Windows taskbar.
use super::{Phase, read, runtime_demo, tray_action, window_action};
use std::time::Duration;
use tauri::Manager;

async fn step(
    app: &tauri::AppHandle,
    name: &'static str,
    check: impl FnOnce(&tauri::AppHandle) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = check(&handle);
        if result.is_ok() {
            println!("SAYWEAVE_ACCEPTANCE {name}: PASS");
        }
        let _ = send.send(result);
    })
    .map_err(|e| e.to_string())?;
    receive.await.map_err(|e| e.to_string())?
}
fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned())
    }
}
fn viewport(app: &tauri::AppHandle, expected: (f64, f64)) -> Result<(), String> {
    let window = app
        .get_webview_window("voice-overlay")
        .ok_or("missing overlay")?;
    let size = window
        .inner_size()
        .map_err(|e| e.to_string())?
        .to_logical::<f64>(window.scale_factor().map_err(|e| e.to_string())?);
    require(
        (size.width - expected.0).abs() < 1.0 && (size.height - expected.1).abs() < 1.0,
        &format!("viewport {}x{} != {expected:?}", size.width, size.height),
    )
}
async fn verify(app: &tauri::AppHandle, menu_ids: Vec<String>) -> Result<(), String> {
    if crate::installer::login_startup::LaunchReason::current()
        == crate::installer::login_startup::LaunchReason::Autostart
    {
        // Observe after initial native/frontend event handling, before any test opens it.
        tokio::time::sleep(Duration::from_millis(250)).await;
        step(app, "autostart-hidden-idle", |app| {
            let main = app.get_webview_window("main").ok_or("missing main")?;
            let snapshot = read(app).map_err(|e| format!("{e:?}"))?;
            require(
                !main.is_visible().map_err(|e| e.to_string())?,
                "autostart main visible",
            )?;
            require(
                !main.is_focused().map_err(|e| e.to_string())?,
                "autostart took focus",
            )?;
            require(
                !snapshot.windows.main
                    && !snapshot.windows.overlay
                    && snapshot.session.phase == Phase::Idle,
                "autostart runtime not hidden/idle",
            )
        })
        .await?;
        let shortcut_deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let status = app
                .state::<crate::shortcuts::ShortcutService>()
                .status()
                .map_err(|e| format!("{e:?}"))?;
            if status.active.is_some() && !status.pending && status.error.is_none() {
                break;
            }
            if std::time::Instant::now() >= shortcut_deadline {
                return Err("hidden launch shortcut initialization incomplete".into());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        println!("SAYWEAVE_ACCEPTANCE autostart-shortcuts-ready: PASS");
        let mut duplicate =
            std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
                .arg("--autostart")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| e.to_string())?;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while duplicate.try_wait().map_err(|e| e.to_string())?.is_none() {
            if std::time::Instant::now() >= deadline {
                let _ = duplicate.kill();
                let _ = duplicate.wait();
                return Err("duplicate autostart did not exit silently".into());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let output = duplicate.wait_with_output().map_err(|e| e.to_string())?;
        require(
            String::from_utf8_lossy(&output.stderr).contains("SAYWEAVE_ALREADY_RUNNING"),
            "duplicate did not reach existing instance lock",
        )?;
        step(app, "autostart-duplicate-keeps-main-hidden", |app| {
            let main = app.get_webview_window("main").ok_or("missing main")?;
            require(
                !main.is_visible().map_err(|e| e.to_string())?,
                "duplicate opened main",
            )?;
            tray_action(app, "open").map_err(|e| format!("{e:?}"))?;
            require(
                main.is_visible().map_err(|e| e.to_string())?
                    && read(app).map_err(|e| format!("{e:?}"))?.windows.main,
                "tray did not open hidden main",
            )
        })
        .await?;
    }
    step(app, "overlay-identity-without-ui-dispatch", |app| {
        let window = app
            .get_webview_window("voice-overlay")
            .ok_or("missing overlay")?;
        let expected = crate::voice_window::instance(&window);
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = send.send(crate::voice_window::instance(&window));
        });
        // The UI deliberately waits. A worker calling Tauri's synchronous hwnd()
        // getter cannot pass; a registered identity lookup must not dispatch here.
        require(
            expected.is_some()
                && receive.recv_timeout(Duration::from_millis(500)).ok() == Some(expected),
            "overlay identity blocks on UI thread",
        )
    })
    .await?;
    step(app, "tray-and-menu", move |app| {
        require(app.tray_by_id("sayweave-tray").is_some(), "missing tray")?;
        require(
            menu_ids == ["open", "voice", "hide", "exit"],
            "menu ID mismatch",
        )
    })
    .await?;
    step(app, "main-minimum-size", |app| {
        let main = app.get_webview_window("main").ok_or("missing main")?;
        main.set_size(tauri::LogicalSize::new(960.0, 640.0))
            .map_err(|e| e.to_string())
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(100)).await;
    step(app, "main-minimum-size-verified", |app| {
        let main = app.get_webview_window("main").ok_or("missing main")?;
        let size = main
            .inner_size()
            .map_err(|e| e.to_string())?
            .to_logical::<f64>(main.scale_factor().map_err(|e| e.to_string())?);
        require(
            (size.width - 960.0).abs() < 1.0 && (size.height - 640.0).abs() < 1.0,
            "minimum main viewport differs",
        )?;
        main.set_size(tauri::LogicalSize::new(1120.0, 760.0))
            .map_err(|e| e.to_string())
    })
    .await?;
    step(app, "hide-recover-no-duplicate", |app| {
        let main = app.get_webview_window("main").ok_or("missing main")?;
        window_action(app, "main", "hide").map_err(|e| format!("{e:?}"))?;
        require(
            !main.is_visible().map_err(|e| e.to_string())?,
            "main did not hide",
        )?;
        tray_action(app, "open").map_err(|e| format!("{e:?}"))?;
        tray_action(app, "open").map_err(|e| format!("{e:?}"))?;
        require(
            main.is_visible().map_err(|e| e.to_string())? && app.webview_windows().len() == 2,
            "recover duplicated or invisible",
        )
    })
    .await?;
    step(app, "overlay-show-hide", |app| {
        let overlay = app
            .get_webview_window("voice-overlay")
            .ok_or("missing overlay")?;
        tray_action(app, "voice").map_err(|e| format!("{e:?}"))?;
        require(
            overlay.is_visible().map_err(|e| e.to_string())?,
            "overlay not visible",
        )?;
        tray_action(app, "hide").map_err(|e| format!("{e:?}"))?;
        require(
            !overlay.is_visible().map_err(|e| e.to_string())?,
            "overlay not hidden",
        )
    })
    .await?;
    step(app, "session-start", |app| {
        tray_action(app, "voice").map_err(|e| format!("{e:?}"))?;
        let main = app.get_webview_window("main").ok_or("missing main")?;
        runtime_demo(app.clone(), main, "start".into(), 0).map_err(|e| format!("{e:?}"))?;
        window_action(app, "main", "hide").map_err(|e| format!("{e:?}"))?;
        Ok(())
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(850)).await;
    step(app, "hidden-main-session-continues", |app| {
        let snapshot = read(app).map_err(|e| format!("{e:?}"))?;
        require(
            snapshot.session.phase == Phase::Recording && !snapshot.windows.main,
            "session did not continue",
        )?;
        let overlay = app
            .get_webview_window("voice-overlay")
            .ok_or("missing overlay")?;
        runtime_demo(app.clone(), overlay, "finish".into(), snapshot.session.id)
            .map_err(|e| format!("{e:?}"))?;
        viewport(app, (160.0, 88.0))?;
        Ok(())
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(400)).await;
    step(app, "processing-shrink-after-transition", |app| {
        viewport(app, (160.0, 88.0))
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(950)).await;
    step(app, "overlay-completes-shared-session", |app| {
        let snapshot = read(app).map_err(|e| format!("{e:?}"))?;
        require(
            snapshot.session.phase == Phase::Completed,
            "completion did not settle",
        )?;
        viewport(app, (160.0, 88.0))?;
        let main = app.get_webview_window("main").ok_or("missing main")?;
        let next = runtime_demo(
            app.clone(),
            main.clone(),
            "start".into(),
            snapshot.session.id,
        )
        .map_err(|e| format!("{e:?}"))?;
        runtime_demo(app.clone(), main, "cancel".into(), next.session.id)
            .map_err(|e| format!("{e:?}"))?;
        Ok(())
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    step(app, "cancel-rejects-late-results", |app| {
        require(
            read(app).map_err(|e| format!("{e:?}"))?.session.phase == Phase::Cancelled,
            "late result changed cancelled session",
        )
    })
    .await?;
    step(app, "failure-retry-reserves-canvas", |app| {
        let main = app.get_webview_window("main").ok_or("missing main")?;
        let id = read(app).map_err(|e| format!("{e:?}"))?.session.id;
        let next = runtime_demo(app.clone(), main.clone(), "start".into(), id)
            .map_err(|e| format!("{e:?}"))?;
        runtime_demo(app.clone(), main.clone(), "fail".into(), next.session.id)
            .map_err(|e| format!("{e:?}"))?;
        viewport(app, (160.0, 88.0))?;
        let retry = runtime_demo(app.clone(), main.clone(), "start".into(), next.session.id)
            .map_err(|e| format!("{e:?}"))?;
        runtime_demo(app.clone(), main, "cancel".into(), retry.session.id)
            .map_err(|e| format!("{e:?}"))?;
        Ok(())
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    step(app, "restart-before-old-shrink", |app| {
        let main = app.get_webview_window("main").ok_or("missing main")?;
        let id = read(app).map_err(|e| format!("{e:?}"))?.session.id;
        runtime_demo(app.clone(), main, "start".into(), id).map_err(|e| format!("{e:?}"))?;
        Ok(())
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(240)).await;
    step(app, "stale-shrink-rejected", |app| {
        viewport(app, (160.0, 88.0))
    })
    .await?;
    tokio::time::sleep(Duration::from_millis(150)).await;
    step(app, "latest-shrink-applied", |app| {
        viewport(app, (160.0, 88.0))
    })
    .await
}
pub fn start(app: tauri::AppHandle, menu_ids: Vec<String>) {
    tauri::async_runtime::spawn(async move {
        let result = verify(&app, menu_ids).await;
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || match result {
            Ok(()) => {
                println!("SAYWEAVE_ACCEPTANCE_PASS");
                let _ = tray_action(&handle, "exit");
            }
            Err(error) => {
                eprintln!("SAYWEAVE_ACCEPTANCE_FAIL {error}");
                handle.exit(1);
            }
        });
    });
}
