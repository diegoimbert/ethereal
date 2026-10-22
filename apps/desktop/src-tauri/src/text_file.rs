//! Saving and opening a small text file the user picks in the native file dialog (theme
//! files: Settings > Appearance). The webview can't reach the file system, and a browser
//! download does nothing in it; the dialog runs here so the UI only ever writes the file the
//! user just chose, never a path of its own.

use std::path::PathBuf;

use tauri_plugin_dialog::{DialogExt, FilePath};

/// Largest file [`text_file_open`] reads (a theme file is a few KB).
const MAX_BYTES: u64 = 1024 * 1024;

fn local(path: FilePath) -> Result<PathBuf, String> {
    path.into_path().map_err(|e| e.to_string())
}

/// Asks where to save `contents` (suggesting `name`, files of `extensions` named `filter`)
/// and writes it there. `None` when the dialog was dismissed; else the path written.
#[tauri::command]
pub async fn text_file_save(
    app: tauri::AppHandle,
    name: String,
    contents: String,
    filter: String,
    extensions: Vec<String>,
) -> Result<Option<String>, String> {
    let exts: Vec<&str> = extensions.iter().map(String::as_str).collect();
    let Some(path) = app
        .dialog()
        .file()
        .set_file_name(&name)
        .add_filter(&filter, &exts)
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = local(path)?;
    std::fs::write(&path, contents).map_err(|e| e.to_string())?;
    Ok(Some(path.display().to_string()))
}

/// Asks for a file (of `extensions`, named `filter`) and reads it as UTF-8 text. `None` when
/// the dialog was dismissed.
#[tauri::command]
pub async fn text_file_open(
    app: tauri::AppHandle,
    filter: String,
    extensions: Vec<String>,
) -> Result<Option<String>, String> {
    let exts: Vec<&str> = extensions.iter().map(String::as_str).collect();
    let Some(path) = app
        .dialog()
        .file()
        .add_filter(&filter, &exts)
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = local(path)?;
    let len = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
    if len > MAX_BYTES {
        return Err(format!("{} is too large ({len} bytes)", path.display()));
    }
    std::fs::read_to_string(&path)
        .map(Some)
        .map_err(|e| e.to_string())
}
