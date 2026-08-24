//! Bytes -> data-URI helpers for tools that return images to the vision model.

use anyhow::{Context, Result};
use base64::Engine;
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, DynamicImage, GenericImageView};

const TOOL_IMAGE_MAX_DIMENSION: u32 = 768;
const TOOL_IMAGE_JPEG_QUALITY: u8 = 65;

/// Sniff the image MIME type from magic bytes. Falls back to `image/png`
/// (a safe default most providers render) when the signature is unknown.
pub fn sniff_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && bytes.len() > 11 && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else if bytes.starts_with(b"BM") {
        "image/bmp"
    } else {
        "image/png"
    }
}

/// Encode raw image bytes into a `data:<mime>;base64,<...>` URI.
pub fn bytes_to_data_uri(bytes: &[u8]) -> String {
    let mime = sniff_mime(bytes);
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{b64}")
}

/// Normalize a tool screenshot before embedding it in the next model request.
pub fn tool_image_to_data_uri(bytes: &[u8]) -> Result<String> {
    let image = image::load_from_memory(bytes).context("decode tool image")?;
    let resized = resize_to_fit(image, TOOL_IMAGE_MAX_DIMENSION);
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, TOOL_IMAGE_JPEG_QUALITY)
        .encode_image(&resized)
        .context("encode tool image as jpeg")?;
    Ok(bytes_to_data_uri(&encoded))
}

fn resize_to_fit(image: DynamicImage, limit: u32) -> DynamicImage {
    let (width, height) = image.dimensions();
    let longest = width.max(height);
    if longest <= limit {
        return image;
    }
    let scale = f64::from(limit) / f64::from(longest);
    let width = (f64::from(width) * scale).round().max(1.0) as u32;
    let height = (f64::from(height) * scale).round().max(1.0) as u32;
    image.resize_exact(width, height, FilterType::Lanczos3)
}

/// Read an image file and return its data URI. Errors if the file cannot be
/// read. Use for local image files and saved screenshots.
pub fn file_to_data_uri(path: &std::path::Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("cannot read image file: {}", path.display()))?;
    Ok(bytes_to_data_uri(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1x1 transparent PNG (smallest valid PNG with IHDR + IDAT + IEND).
    const PNG_BYTES: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
        0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
    ];

    // Minimal JPEG with SOI + marker.
    const JPEG_BYTES: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];

    const GIF_BYTES: &[u8] = b"GIF89a";
    const WEBP_BYTES: &[u8] = &[b'R', b'I', b'F', b'F', 0, 0, 0, 0, b'W', b'E', b'B', b'P'];
    const BMP_BYTES: &[u8] = b"BM";

    #[test]
    fn sniff_mime_recognizes_png() {
        assert_eq!(sniff_mime(PNG_BYTES), "image/png");
    }

    #[test]
    fn sniff_mime_recognizes_jpeg() {
        assert_eq!(sniff_mime(JPEG_BYTES), "image/jpeg");
    }

    #[test]
    fn sniff_mime_recognizes_gif() {
        assert_eq!(sniff_mime(GIF_BYTES), "image/gif");
    }

    #[test]
    fn sniff_mime_recognizes_webp() {
        assert_eq!(sniff_mime(WEBP_BYTES), "image/webp");
    }

    #[test]
    fn sniff_mime_recognizes_bmp() {
        assert_eq!(sniff_mime(BMP_BYTES), "image/bmp");
    }

    #[test]
    fn sniff_mime_defaults_unknown_to_png() {
        assert_eq!(sniff_mime(b"not an image at all"), "image/png");
        assert_eq!(sniff_mime(&[]), "image/png");
    }

    #[test]
    fn bytes_to_data_uri_produces_png_uri() {
        let uri = bytes_to_data_uri(PNG_BYTES);
        assert!(
            uri.starts_with("data:image/png;base64,"),
            "expected png data uri, got: {uri}"
        );
        // Body must be valid base64 of the input bytes.
        let b64 = &uri["data:image/png;base64,".len()..];
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("data uri body must be base64");
        assert_eq!(decoded, PNG_BYTES);
    }

    #[test]
    fn bytes_to_data_uri_produces_jpeg_uri() {
        let uri = bytes_to_data_uri(JPEG_BYTES);
        assert!(
            uri.starts_with("data:image/jpeg;base64,"),
            "expected jpeg data uri, got: {uri}"
        );
    }
}
