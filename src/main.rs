use tauri::command;
use std::fs;

#[command]
async fn open_file(app: tauri::AppHandle) -> Result<Option<serde_json::Value>, String> {
    let file_path: Option<std::path::PathBuf> = app
        .dialog()
        .file()
        .add_filter("Text Files", &["txt", "md", "rs", "js", "html", "css"])
        .pick_file()
        .await
        .ok()
        .flatten();

    match file_path {
        Some(path) => {
            match fs::read_to_string(&path) {
                Ok(content) => {
                    let path_str = path.to_string_lossy().to_string();
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
async fn save_file(app: tauri::AppHandle, content: String, path: Option<String>) -> Result<Option<serde_json::Value>, String> {
    let file_path = if let Some(p) = path {
        std::path::PathBuf::from(p)
    } else {
        let file_path: Option<std::path::PathBuf> = app
            .dialog()
            .file()
            .add_filter("Text Files", &["txt", "md", "rs", "js", "html", "css"])
            .save_file()
            .await
            .ok()
            .flatten();
        match file_path {
            Some(p) => p,
            None => return Ok(None),
        }
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
        .invoke_handler(tauri::generate_handler![open_file, save_file])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
