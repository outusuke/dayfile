mod store;

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use store::{Entry, Meta};
use tauri::{Manager, State};

struct AppState {
    dir: Mutex<PathBuf>,
    config: PathBuf,
}

impl AppState {
    // Recover from a poisoned lock so one panicked thread doesn't kill every later command.
    fn dir(&self) -> PathBuf {
        self.dir.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

// Android/media/<package> stays reachable from file managers and other apps; Android/data doesn't on Android 11+.
#[cfg(target_os = "android")]
fn android_media_journal(docs: &std::path::Path) -> Option<PathBuf> {
    let (root, rest) = docs.to_str()?.split_once("/Android/data/")?;
    let package = rest.split('/').next()?;
    Some(PathBuf::from(format!("{root}/Android/media/{package}/Journal")))
}

#[cfg(target_os = "android")]
fn adopt_old_entries(old: &std::path::Path, new: &std::path::Path) {
    let empty = fs::read_dir(new).map(|mut d| d.next().is_none()).unwrap_or(false);
    if !empty {
        return;
    }
    let Ok(entries) = fs::read_dir(old) else { return };
    for e in entries.flatten() {
        let from = e.path();
        if from.is_file() && fs::copy(&from, new.join(e.file_name())).is_ok() {
            let _ = fs::remove_file(from);
        }
    }
}

fn default_dir(app: &tauri::App) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let docs = app.path().document_dir().ok();
    #[cfg(target_os = "android")]
    {
        if let Some(docs) = &docs {
            if let Some(media) = android_media_journal(docs) {
                if store::ensure_writable(&media).is_ok() {
                    adopt_old_entries(&docs.join("Journal"), &media);
                    return Ok(media);
                }
            }
        }
    }
    // app-data fallback is for platforms without a Documents folder
    Ok(match docs {
        Some(d) => d.join("Journal"),
        None => app.path().app_data_dir()?.join("Journal"),
    })
}

#[tauri::command]
async fn journal_dir(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.dir().to_string_lossy().into_owned())
}

#[tauri::command]
async fn set_journal_dir(state: State<'_, AppState>, path: String) -> Result<String, String> {
    let dir = PathBuf::from(path);
    store::ensure_writable(&dir)?;
    store::save_dir(&state.config, &dir)?;
    *state.dir.lock().unwrap_or_else(PoisonError::into_inner) = dir.clone();
    Ok(dir.to_string_lossy().into_owned())
}

#[tauri::command]
async fn list_entries(state: State<'_, AppState>) -> Result<Vec<Entry>, String> {
    store::list(&state.dir())
}

#[tauri::command]
async fn read_entry(state: State<'_, AppState>, date: String) -> Result<Option<Entry>, String> {
    store::read(&state.dir(), &date)
}

#[tauri::command]
async fn write_entry(
    state: State<'_, AppState>,
    date: String,
    body: String,
    meta: Meta,
    expected_rev: Option<String>,
    force: Option<bool>,
) -> Result<Option<Entry>, String> {
    store::write(&state.dir(), &date, &body, &meta, expected_rev.as_deref(), force.unwrap_or(false))
}

#[tauri::command]
async fn delete_entry(state: State<'_, AppState>, date: String) -> Result<(), String> {
    store::delete(&state.dir(), &date)
}

#[tauri::command]
async fn export_all(state: State<'_, AppState>, path: String, format: String) -> Result<usize, String> {
    store::export(&state.dir(), &PathBuf::from(path), &format)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            fs::create_dir_all(&config_dir)?;
            let config = config_dir.join("config.json");

            let dir = match store::load_dir(&config) {
                Some(d) => d,
                None => default_dir(app)?,
            };
            if let Err(err) = fs::create_dir_all(&dir) {
                eprintln!("[dayfile] couldn't create {}: {err}", dir.display());
            }
            app.manage(AppState { dir: Mutex::new(dir), config });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            journal_dir,
            set_journal_dir,
            list_entries,
            read_entry,
            write_entry,
            delete_entry,
            export_all,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Dayfile");
}
