use tauri::command;
use tauri_plugin_dialog::DialogExt;
use notepad_extra::{read_file_at, write_file_at};

#[command]
async fn open_file(app: tauri::AppHandle) -> Result<Option<serde_json::Value>, String> {
    // Because this command is `async`, it runs on a background thread.
    // blocking_pick_file will safely pause this thread without freezing the UI!
    let file_path = app.dialog()
        .file()
        .add_filter("Text Files", &["txt", "md", "rs", "js", "html", "css"])
        .blocking_pick_file();

    match file_path {
        Some(fp) => {
            let path_buf = fp.into_path().map_err(|e| e.to_string())?;
            read_file_at(&path_buf).map(Some)
        }
        None => Ok(None),
    }
}

#[command]
async fn save_file(app: tauri::AppHandle, content: String, path: Option<String>) -> Result<Option<serde_json::Value>, String> {
    let file_path = if let Some(p) = path {
        std::path::PathBuf::from(p)
    } else {
        let fp = app.dialog()
            .file()
            .add_filter("Text Files", &["txt", "md", "rs", "js", "html", "css"])
            .blocking_save_file();
            
        match fp {
            Some(fp) => fp.into_path().map_err(|e| e.to_string())?,
            None => return Ok(None), 
        }
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