//! Image loading utilities for TUI multimodal input. Converts image files
//! pasted/dragged into the composer into `data:image/<fmt>;base64,...` URIs
//! suitable for `SessionInput.images` / `ContentBlock::Image`.
//!
//! Logic mirrors the CLI's `load_image_data_uris` / `mime_from_ext` in
//! `crates/cli/src/run.rs` but is kept TUI-local to avoid coupling the CLI
//! crate as a dependency of the TUI.

use std::path::Path;

use base64::Engine;

/// File extensions recognised as image attachments.
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// Return true when `path` has a recognised image file extension.
pub fn is_image_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| {
            let lower = s.to_ascii_lowercase();
            IMAGE_EXTS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

/// Extract the file name (last component) from a path string for display.
pub fn extract_filename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string()
}

/// Map a file extension to an image MIME type. Unknown extensions fall back
/// to `image/png`, the most widely supported default for vision endpoints.
pub fn mime_from_ext(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        _ => "image/png",
    }
}

/// Read an image file and encode it as a `data:{mime};base64,{...}` URI.
/// Returns an error message string on read failure.
pub fn load_image_data_uri(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mime = mime_from_ext(path);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{mime};base64,{b64}"))
}

/// Try to load a pasted string as an image file path. Returns `Some((data_uri, filename))`
/// when the string is a readable image file, `None` otherwise.
pub fn try_load_image(pasted: &str, workdir: &Path) -> Option<(String, String)> {
    let trimmed = pasted.trim();
    if !is_image_path(trimmed) {
        return None;
    }
    let path = if Path::new(trimmed).is_absolute() {
        Path::new(trimmed).to_path_buf()
    } else {
        workdir.join(trimmed)
    };
    match path.canonicalize() {
        Ok(full) => {
            let data_uri = load_image_data_uri(&full).ok()?;
            let filename = extract_filename(trimmed);
            Some((data_uri, filename))
        }
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_image_path_recognises_common_extensions() {
        assert!(is_image_path("photo.png"));
        assert!(is_image_path("photo.jpg"));
        assert!(is_image_path("photo.JPEG"));
        assert!(is_image_path("photo.GIF"));
        assert!(is_image_path("photo.webp"));
        assert!(is_image_path("photo.bmp"));
        assert!(!is_image_path("readme.md"));
        assert!(!is_image_path("script.sh"));
        assert!(!is_image_path("noext"));
    }

    #[test]
    fn extract_filename_returns_last_component() {
        assert_eq!(extract_filename("/tmp/photos/cat.png"), "cat.png");
        assert_eq!(extract_filename("cat.png"), "cat.png");
        assert_eq!(extract_filename("./a/b/c.jpg"), "c.jpg");
    }

    #[test]
    fn mime_from_ext_maps_correctly() {
        assert_eq!(mime_from_ext(Path::new("a.png")), "image/png");
        assert_eq!(mime_from_ext(Path::new("a.jpg")), "image/jpeg");
        assert_eq!(mime_from_ext(Path::new("a.jpeg")), "image/jpeg");
        assert_eq!(mime_from_ext(Path::new("a.gif")), "image/gif");
        assert_eq!(mime_from_ext(Path::new("a.webp")), "image/webp");
        assert_eq!(mime_from_ext(Path::new("a.bmp")), "image/bmp");
        assert_eq!(mime_from_ext(Path::new("a.txt")), "image/png");
    }

    #[test]
    fn load_image_data_uri_generates_valid_uri() {
        // Minimal 1x1 PNG
        let png_bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), png_bytes).unwrap();
        let uri = load_image_data_uri(tmp.path()).unwrap();
        assert!(uri.starts_with("data:image/png;base64,"));
        // base64 of the 8 bytes above
        assert!(uri.contains("iVBORw0KGgo"));
    }

    #[test]
    fn load_image_data_uri_errors_on_missing_file() {
        let result = load_image_data_uri(Path::new("/nonexistent/file.png"));
        assert!(result.is_err());
    }

    #[test]
    fn try_load_image_returns_none_for_non_image() {
        let workdir = Path::new(".");
        assert!(try_load_image("readme.md", workdir).is_none());
        assert!(try_load_image("no extension", workdir).is_none());
    }

    #[test]
    fn try_load_image_loads_image_file() {
        let png_bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let tmp = tempfile::NamedTempFile::with_suffix(".png").unwrap();
        std::fs::write(tmp.path(), png_bytes).unwrap();
        let path_str = tmp.path().to_str().unwrap();
        let result = try_load_image(path_str, Path::new("."));
        assert!(result.is_some());
        let (uri, name) = result.unwrap();
        assert!(uri.starts_with("data:image/png;base64,"));
        assert!(name.ends_with(".png"));
    }
}
