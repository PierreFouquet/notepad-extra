use tauri::{command, State};
use std::fs;
use std::sync::Mutex;

// Define a struct to hold app state if needed
struct AppState {
    // For now, empty
}

#[command]
async fn open_file() -> Result<Option<serde_json::Value>, String> {
    use tauri::api::dialog::FileDialogBuilder;

    let (tx, rx) = std::sync::mpsc::channel();

    FileDialogBuilder::new()
        .add_filter("Text Files", &["txt", "md", "rs", "js", "html", "css"])
        .pick_file(move |file_path| {
            tx.send(file_path).unwrap();
        });

    match rx.recv().unwrap() {
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
async fn save_file(content: String, path: Option<String>) -> Result<Option<serde_json::Value>, String> {
    use tauri::api::dialog::FileDialogBuilder;

    let file_path = if let Some(p) = path {
        std::path::PathBuf::from(p)
    } else {
        let (tx, rx) = std::sync::mpsc::channel();
        FileDialogBuilder::new()
            .add_filter("Text Files", &["txt", "md", "rs", "js", "html", "css"])
            .save_file(move |file_path| {
                tx.send(file_path).unwrap();
            });
        match rx.recv().unwrap() {
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
