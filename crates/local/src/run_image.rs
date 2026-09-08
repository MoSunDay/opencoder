//! `--image` attachment loading helpers, extracted from `run.rs` to keep
//! that file under the line budget. Reads image files into base64 data URIs
//! suitable for vision-capable chat endpoints.

use std::path::Path;

use anyhow::{Context, Result};
use base64::Engine as _;

/// Read each `--image` file path into a `data:image/<fmt>;base64,<...>` URI
/// suitable for attachment to a chat message.
pub(crate) fn load_image_data_uris(paths: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let path = Path::new(p);
        let bytes =
            std::fs::read(path).with_context(|| format!("--image {p}: cannot read file"))?;
        let mime = mime_from_ext(path);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        out.push(format!("data:{mime};base64,{b64}"));
    }
    Ok(out)
}

/// Map a file extension to an image MIME type. Unknown extensions fall back to
/// `image/png`, the most widely supported default for vision endpoints.
fn mime_from_ext(path: &Path) -> &'static str {
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
