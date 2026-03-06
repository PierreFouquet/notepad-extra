use tauri::command;
use tauri_plugin_dialog::DialogExt;
use std::time::Duration;
use std::path::Path;
use notepad_extra::{read_file_at, write_file_at};

#[command]
fn open_file(app: tauri::AppHandle) -> Result<Option<serde_json::Value>, String> {
    let (tx, rx) = std::sync::mpsc::channel();

    app.dialog()
        .file()
        .add_filter("Text Files", &["txt", "md", "rs", "js", "html", "css"])
        .pick_file(move |file_path| {
            let _ = tx.send(file_path);
        });

    let file_path = rx
        .recv_timeout(Duration::from_secs(30))
        .map_err(|e| format!("no response from dialog: {}", e))?;

    match file_path {
        Some(fp) => {
            // Convert the plugin FilePath into a PathBuf when possible
            let path_buf = fp.into_path().map_err(|e| e.to_string())?;
            read_file_at(&path_buf).map(Some)
        }
        None => Ok(None),
    }
}

#[command]
fn save_file(app: tauri::AppHandle, content: String, path: Option<String>) -> Result<Option<serde_json::Value>, String> {
    let file_path = if let Some(p) = path {
        std::path::PathBuf::from(p)
    } else {
        let (tx, rx) = std::sync::mpsc::channel();
        app.dialog()
            .file()
            .add_filter("Text Files", &["txt", "md", "rs", "js", "html", "css"])
            .save_file(move |file_path| {
                let _ = tx.send(file_path);
            });
        let fp = match rx
            .recv_timeout(Duration::from_secs(30))
            .map_err(|e| format!("no response from dialog: {}", e))? {
            Some(fp) => fp,
            None => return Ok(None),
        };
        // convert plugin FilePath into PathBuf
        fp.into_path().map_err(|e| e.to_string())?
    };

    write_file_at(&content, &file_path).map(Some)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![open_file, save_file])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

