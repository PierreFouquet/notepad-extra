use tauri::command;
use tauri_plugin_dialog::DialogExt;
use std::fs;

#[command]
fn open_file(app: tauri::AppHandle) -> Result<Option<serde_json::Value>, String> {
    let (tx, rx) = std::sync::mpsc::channel();

    app.dialog()
        .file()
        .add_filter("Text Files", &["txt", "md", "rs", "js", "html", "css"])
        .pick_file(move |file_path| {
            let _ = tx.send(file_path);
        });

    let file_path = rx.recv().map_err(|e| e.to_string())?;

    match file_path {
        Some(fp) => {
            // Convert the plugin FilePath into a PathBuf when possible
            let path_buf = fp.into_path().map_err(|e| e.to_string())?;
            match fs::read_to_string(&path_buf) {
                Ok(content) => {
                    let path_str = path_buf.to_string_lossy().to_string();
                    Ok(Some(serde_json::json!({
                        "path": path_str,
                        "content": content
                    })))
                }
                Err(e) => Err(format!("Failed to read file: {}", e)),
            }
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
        let fp = match rx.recv().map_err(|e| e.to_string())? {
            Some(fp) => fp,
            None => return Ok(None),
        };
        // convert plugin FilePath into PathBuf
        fp.into_path().map_err(|e| e.to_string())?
    };

    match fs::write(&file_path, content) {
        Ok(_) => {
            let path_str = file_path.to_string_lossy().to_string();
            Ok(Some(serde_json::json!({
                "path": path_str
            })))
        }
        Err(e) => Err(format!("Failed to save file: {}", e)),
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![open_file, save_file])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
