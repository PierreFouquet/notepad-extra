use tauri::command;
use tauri_plugin_dialog::DialogExt;
use notepad_extra::{read_file_at, write_file_at};

/// Extensions offered in the "Text & Code" dialog filter.
const TEXT_EXTS: &[&str] = &[
    "txt", "md", "markdown", "rs", "js", "mjs", "cjs", "ts", "jsx", "json",
    "py", "pyw", "c", "h", "cpp", "cc", "cxx", "hpp", "hxx", "java",
    "html", "htm", "xml", "svg", "xaml", "css", "sh", "bash", "zsh",
    "yml", "yaml", "log", "ini", "toml", "cfg", "conf",
];

#[command]
async fn open_file(app: tauri::AppHandle) -> Result<Option<serde_json::Value>, String> {
    // Because this command is `async`, it runs on a background thread.
    // blocking_pick_file will safely pause this thread without freezing the UI!
    let file_path = app.dialog()
        .file()
        .add_filter("Text & Code Files", TEXT_EXTS)
        .add_filter("All Files", &["*"])
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
        .add_filter("Text & Code Files", TEXT_EXTS)
        .add_filter("All Files", &["*"])
        .blocking_save_file()
        .and_then(|fp| fp.into_path().ok())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![open_file, save_file, save_file_as])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
