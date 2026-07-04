use notepad_extra::{is_safe_external_url, read_file_at, write_file_at};
use tauri::command;
use tauri_plugin_dialog::DialogExt;

#[command]
async fn open_file(app: tauri::AppHandle) -> Result<Option<serde_json::Value>, String> {
    // Because this command is `async`, it runs on a background thread.
    // blocking_pick_file will safely pause this thread without freezing the UI!
    // No filter is applied, so every file is shown (including extension-less
    // ones like `Dockerfile` that a glob filter could never match).
    let file_path = app.dialog().file().blocking_pick_file();

    match file_path {
        Some(fp) => {
            let path_buf = fp.into_path().map_err(|e| e.to_string())?;
            read_file_at(&path_buf).map(Some)
        }
        None => Ok(None),
    }
}

/// Read a file at an explicit `path` and return `{ path, content }`.
///
/// Unlike `open_file`, this takes a path directly rather than showing a picker,
/// so the frontend can open files dragged and dropped onto the window.
#[command]
async fn read_file(path: String) -> Result<serde_json::Value, String> {
    read_file_at(std::path::Path::new(&path))
}

#[command]
async fn save_file(
    app: tauri::AppHandle,
    content: String,
    path: Option<String>,
) -> Result<Option<serde_json::Value>, String> {
    let file_path = if let Some(p) = path {
        std::path::PathBuf::from(p)
    } else {
        match prompt_save_path(&app) {
            Some(fp) => fp,
            None => return Ok(None),
        }
    };

    write_file_at(&content, &file_path).map(Some)
}

/// Always prompts for a destination, regardless of any existing path ("Save As").
#[command]
async fn save_file_as(
    app: tauri::AppHandle,
    content: String,
) -> Result<Option<serde_json::Value>, String> {
    match prompt_save_path(&app) {
        Some(fp) => write_file_at(&content, &fp).map(Some),
        None => Ok(None),
    }
}

/// Show the native "save file" dialog and return the chosen path, if any.
fn prompt_save_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.dialog()
        .file()
        .blocking_save_file()
        .and_then(|fp| fp.into_path().ok())
}

/// The application version, taken from Cargo.toml at compile time so the About
/// dialog never drifts from the real build.
#[command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Hand an https URL to the operating system so it opens in the user's own
/// browser. The app itself makes no network requests — this only launches the
/// platform URL handler. Only `https` links are accepted, and the URL is passed
/// as a single argument (never through a shell), so it cannot inject commands.
#[command]
fn open_external(url: String) -> Result<(), String> {
    if !is_safe_external_url(&url) {
        return Err("refusing to open unsafe or non-https URL".into());
    }

    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(&url);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(&url);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        // rundll32 passes the URL straight to the default handler without a shell.
        let mut c = std::process::Command::new("rundll32.exe");
        c.arg("url.dll,FileProtocolHandler").arg(&url);
        c
    };

    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to open URL: {}", e))
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            open_file,
            read_file,
            save_file,
            save_file_as,
            app_version,
            open_external
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
