use std::fs;
use std::path::Path;

fn main() {
    let icon_dir = Path::new("icons");
    if !icon_dir.exists() {
        let _ = fs::create_dir_all(icon_dir);
    }
    let icon_path = icon_dir.join("icon.png");

    // If an icon exists, try to load and convert it to RGBA8 PNG.
    if icon_path.exists() && fs::metadata(&icon_path).map(|m| m.len()).unwrap_or(0) > 0 {
        if let Err(e) = try_convert_to_rgba(&icon_path) {
            eprintln!("warning: failed to convert icon to RGBA: {}", e);
        }
    } else {
        // Write a 1x1 transparent RGBA PNG as fallback
        let png: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0xE2, 0x26, 0x05, 0x9B, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        let _ = fs::write(&icon_path, png);
    }

    tauri_build::build()
}

fn try_convert_to_rgba(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use image::io::Reader as ImageReader;
    use image::ImageFormat;

    let img = ImageReader::open(path)?.with_guessed_format()?.decode()?;
    let rgba = img.to_rgba8();
    // Save back as PNG (this will be 8-bit RGBA)
    rgba.save_with_format(path, ImageFormat::Png)?;
    Ok(())
}