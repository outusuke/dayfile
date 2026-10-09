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

            // App-data fallback is for Android, which has no Documents folder.
            let dir = store::load_dir(&config)
                .or_else(|| app.path().document_dir().ok().map(|d| d.join("Journal")))
                .unwrap_or(app.path().app_data_dir()?.join("Journal"));
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
