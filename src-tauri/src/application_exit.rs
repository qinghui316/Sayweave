//! Shared producer-first shutdown for tray exit and verified update installation.
use tauri::Manager;
pub async fn stop(app: &tauri::AppHandle, archive_watermark: usize) -> Result<(), String> {
    app.state::<crate::shortcuts::ShortcutService>()
        .shutdown(app);
    app.state::<crate::dictation::DictationCoordinator>()
        .shutdown();
    app.state::<crate::speech::SpeechService>().shutdown();
    // Retire observation synchronously before stopping learning, preserving its final evidence.
    #[cfg(windows)]
    crate::windows_input::composition::stop_observer()
        .await
        .map_err(|e| e.code)?;
    app.state::<crate::windows_input::WindowsInputService>()
        .shutdown();
    app.state::<crate::shortcuts::ShortcutService>()
        .drain(app)
        .await
        .map_err(|e| e.code)?;
    app.state::<crate::speech::SpeechService>()
        .drain()
        .await
        .map_err(|e| e.code)?;
    app.state::<crate::dictation::DictationCoordinator>()
        .drain()
        .await;
    app.state::<crate::windows_input::WindowsInputService>()
        .drain()
        .await;
    crate::archive::drain_since(archive_watermark)
        .await
        .map_err(str::to_owned)?;
    app.state::<crate::language_learning::LanguageLearning>()
        .shutdown();
    app.state::<crate::models::ModelService>().begin_shutdown();
    app.state::<crate::language_learning::LanguageLearning>()
        .drain()
        .await
        .map_err(|e| e.code)?;
    app.state::<crate::models::ModelService>()
        .finish_shutdown()
        .await
        .map_err(|e| e.code)?;
    app.state::<crate::data::DataWorker>()
        .seal_and_drain()
        .await
        .map_err(|e| e.code)?;
    Ok(())
}
