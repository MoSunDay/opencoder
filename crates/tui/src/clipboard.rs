//! System clipboard image reading for the TUI composer. `arboard` provides
//! cross-platform clipboard access (X11/Wayland/macOS/Windows); on a headless
//! box with no display server it returns a graceful error (handled as None).

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

/// Read the system clipboard's image, if any, and return it as a PNG data URI.
/// Returns None when there is no image, no clipboard, or no display server
/// (headless). Call from a blocking thread (e.g. `tokio::task::spawn_blocking`).
pub fn clipboard_image_data_uri() -> Option<String> {
    let mut cb = arboard::Clipboard::new().ok()?;
    let img = cb.get_image().ok()?;
    encode_rgba_png(img.bytes.as_ref(), img.width, img.height)
}

/// Encode raw RGBA pixels as a `data:image/png;base64,...` URI. Pure and
/// unit-testable. Returns None if the buffer size does not match width*height*4.
fn encode_rgba_png(rgba: &[u8], width: usize, height: usize) -> Option<String> {
    let buf = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(
        width as u32,
        height as u32,
        rgba.to_vec(),
    )?;
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(buf)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(format!("data:image/png;base64,{}", STANDARD.encode(&out)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encode_rgba_png_roundtrips_as_png_data_uri() {
        // 2x2 fully-opaque red RGBA image.
        let px = |r, g, b| [r, g, b, 255u8];
        let rgba: Vec<u8> = [px(255, 0, 0), px(255, 0, 0), px(255, 0, 0), px(255, 0, 0)].concat();
        let uri = encode_rgba_png(&rgba, 2, 2).expect("encodes");
        assert!(uri.starts_with("data:image/png;base64,"));
        let b64 = &uri["data:image/png;base64,".len()..];
        let bytes = STANDARD.decode(b64).unwrap();
        // PNG signature.
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        // IHDR width/height = 2/2 (big-endian u32 at offset 16 and 20).
        assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 2);
        assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 2);
    }

    #[test]
    fn encode_rgba_png_rejects_mismatched_size() {
        assert!(encode_rgba_png(&[0; 3], 1, 1).is_none()); // 3 bytes != 1*1*4
    }
}
