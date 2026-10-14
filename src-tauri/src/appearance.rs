use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};
use tauri::{Emitter, Manager};

#[derive(Deserialize)]
pub struct Theme {
    pub id: String,
    pub scheme: String,
    pub vars: BTreeMap<String, String>,
}
pub static THEMES: LazyLock<Vec<Theme>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../src/design/themes.json"))
        .expect("bundled theme catalogue")
});
pub fn theme(id: &str) -> &'static Theme {
    THEMES.iter().find(|t| t.id == id).unwrap_or(&THEMES[0])
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Appearance {
    pub theme_id: String,
    pub revision: u64,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme_id: "light".into(),
            revision: 1,
        }
    }
}
pub struct AppearanceService {
    path: PathBuf,
    value: Mutex<Appearance>,
}
impl AppearanceService {
    pub fn new(root: &Path) -> Self {
        let path = root.join("appearance.json");
        let value = read_file(&path).unwrap_or_default();
        Self {
            path,
            value: Mutex::new(value),
        }
    }
    pub fn read(&self) -> Result<Appearance, String> {
        let mut value = self.value.lock().map_err(|_| "unavailable")?;
        *value = read_file(&self.path)?;
        Ok(value.clone())
    }
    pub fn startup(&self) -> Appearance {
        self.value.lock().expect("appearance startup").clone()
    }
    fn save(&self, requested: Appearance) -> Result<Appearance, String> {
        if requested.theme_id != theme(&requested.theme_id).id {
            return Err("invalidAppearance".into());
        }
        let mut current = self.value.lock().map_err(|_| "unavailable")?;
        if requested.revision != current.revision {
            return Err("conflict".into());
        }
        if requested.theme_id == current.theme_id {
            return Ok(current.clone());
        }
        let next = Appearance {
            theme_id: requested.theme_id,
            revision: current
                .revision
                .checked_add(1)
                .filter(|n| *n <= 9_007_199_254_740_991)
                .ok_or("invalidRevision")?,
        };
        write_file(
            &self.path,
            &serde_json::to_vec(&next).map_err(|_| "appearanceWrite")?,
        )?;
        *current = next.clone();
        Ok(next)
    }
}
fn read_file(path: &Path) -> Result<Appearance, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Appearance::default()),
        Err(_) => return Err("appearanceRead".into()),
    };
    if bytes.len() > 4096 {
        return Err("invalidAppearance".into());
    }
    let mut value: Appearance = serde_json::from_slice(&bytes).map_err(|_| "invalidAppearance")?;
    if value.revision == 0 || value.revision > 9_007_199_254_740_991 {
        return Err("invalidAppearance".into());
    }
    value.theme_id = theme(&value.theme_id).id.clone();
    Ok(value)
}
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_file_name(format!("appearance-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| "appearanceWrite")?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "appearanceWrite")?;
        drop(file);
        #[cfg(windows)]
        unsafe {
            use std::os::windows::ffi::OsStrExt;
            use windows::{
                Win32::Storage::FileSystem::{
                    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
                },
                core::PCWSTR,
            };
            let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
            let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
            .map_err(|_| "appearanceWrite")?;
        }
        #[cfg(not(windows))]
        fs::rename(&temporary, path).map_err(|_| "appearanceWrite")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
pub fn bootstrap(value: &Appearance) -> String {
    // All CSS is bundled data, not user-supplied markup. Both windows consume this same snapshot.
    let current = theme(&value.theme_id);
    format!(
        "window.__SAYWEAVE_APPEARANCE__={};",
        serde_json::to_string(value).unwrap()
    ) + &format!(
        "(()=>{{const t={};const apply=()=>{{const r=document.documentElement;if(!r)return;for(const [k,v] of Object.entries(t.vars))r.style.setProperty(k,v);r.dataset.theme=t.id;r.style.colorScheme=t.scheme;}};apply();document.addEventListener('DOMContentLoaded',apply,{{once:true}});}})();",
        serde_json::json!({"id":current.id,"scheme":current.scheme,"vars":current.vars})
    )
}
pub fn background(id: &str) -> tauri::window::Color {
    let vars = &theme(id).vars;
    let mut value = vars["--surface-canvas"].as_str();
    for _ in 0..8 {
        let Some(key) = value.strip_prefix("var(").and_then(|v| v.strip_suffix(')')) else {
            break;
        };
        value = vars.get(key).expect("bundled color reference");
    }
    let n = u32::from_str_radix(value.trim_start_matches('#'), 16).expect("theme background RGB");
    tauri::window::Color((n >> 16) as u8, (n >> 8) as u8, n as u8, 255)
}
fn allowed(label: &str, write: bool) -> Result<(), String> {
    if label == "main" || !write && label == "voice-overlay" {
        Ok(())
    } else {
        Err("forbidden".into())
    }
}
#[tauri::command]
pub fn appearance_read(
    window: tauri::WebviewWindow,
    service: tauri::State<'_, AppearanceService>,
) -> Result<Appearance, String> {
    allowed(window.label(), false)?;
    service.read()
}
#[tauri::command]
pub fn appearance_save(
    window: tauri::WebviewWindow,
    service: tauri::State<'_, AppearanceService>,
    value: Appearance,
) -> Result<Appearance, String> {
    allowed(window.label(), true)?;
    let value = service.save(value)?;
    let app = window.app_handle().clone();
    // Window work runs after releasing the preference lock, and reads the
    // latest committed choice so queued updates cannot restore an older color.
    let _ = window.run_on_main_thread(move || {
        let current = app.state::<AppearanceService>().startup();
        if let Some(main) = app.get_webview_window("main") {
            let _ = main.set_background_color(Some(background(&current.theme_id)));
        }
    });
    for label in ["main", "voice-overlay"] {
        let _ = window.emit_to(label, "appearance:changed", &value);
    }
    Ok(value)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_preset_bootstraps_both_windows_and_resolves_native_background() {
        for preset in THEMES.iter() {
            let appearance = Appearance {
                theme_id: preset.id.clone(),
                revision: 1,
            };
            let _ = background(&preset.id);
            let script = bootstrap(&appearance);
            assert!(script.contains(&format!("\"themeId\":\"{}\"", preset.id)));
            assert!(script.contains("--voice-fg"));
        }
        assert_eq!(THEMES[0].id, "light");
    }
    #[test]
    fn presets_persist_and_reject_stale_updates() {
        let dir = tempfile::tempdir().unwrap();
        let service = AppearanceService::new(dir.path());
        let first = service.read().unwrap();
        let saved = service
            .save(Appearance {
                theme_id: "graphite".into(),
                ..first.clone()
            })
            .unwrap();
        assert_eq!(AppearanceService::new(dir.path()).read().unwrap(), saved);
        assert!(service.save(first).is_err());
        assert!(allowed("voice-overlay", true).is_err());
        assert!(allowed("voice-overlay", false).is_ok());
    }
    #[test]
    fn unknown_theme_falls_back_without_rewriting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("appearance.json");
        let bytes = br#"{"themeId":"future","revision":9}"#;
        fs::write(&path, bytes).unwrap();
        assert_eq!(read_file(&path).unwrap().theme_id, "light");
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    #[test]
    fn failed_write_keeps_confirmed_choice() {
        let dir = tempfile::tempdir().unwrap();
        let service = AppearanceService::new(&dir.path().join("missing"));
        assert!(
            service
                .save(Appearance {
                    theme_id: "graphite".into(),
                    revision: 1
                })
                .is_err()
        );
        assert_eq!(service.startup(), Appearance::default());
    }
}
