//! Half-block image rendering for the TUI chat transcript. Decodes image
//! bytes, scales to terminal width, and maps each 2-pixel-row to a single
//! character cell using Unicode half-block characters with foreground/background Color::Rgb.
//!
//! For data URIs (data:image/<fmt>;base64,...) the raw bytes are decoded
//! in-process. HTTP(S) URLs cannot be fetched synchronously, so those return
//! an empty Vec (callers fall back to a placeholder).

use base64::Engine;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Decode a data URI (`data:image/<fmt>;base64,...`) to raw bytes.
/// Returns `None` for non-data URIs (http(s)://) or malformed base64.
pub fn decode_data_uri(uri: &str) -> Option<Vec<u8>> {
    let prefix = "data:";
    let rest = uri.strip_prefix(prefix)?;
    let comma = rest.find(',')?;
    let meta = &rest[..comma];
    let payload = &rest[comma + 1..];
    if !meta.contains("base64") {
        return None;
    }
    base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()
}

/// Render image bytes as half-block `Line`s fitting within `max_width` cells.
/// Each output line represents 2 pixel rows: the upper pixel row maps to the
/// cell's foreground (▀ = upper pixel), the lower to background (▄ = lower).
/// Returns an empty `Vec` on decode failure or zero-size images.
pub fn render_image_halfblock(data: &[u8], max_width: u16) -> Vec<Line<'static>> {
    let max_w = max_width.max(1) as u32;
    let img = match image::load_from_memory(data) {
        Ok(img) => img,
        Err(_) => return Vec::new(),
    };
    render_dynamic_image(&img, max_w)
}

/// Render a `DynamicImage` as half-block lines.
fn render_dynamic_image(img: &image::DynamicImage, max_w: u32) -> Vec<Line<'static>> {
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let scale = if w > max_w {
        max_w as f64 / w as f64
    } else {
        1.0
    };
    let new_w = (w as f64 * scale).round().max(1.0) as u32;
    let new_h = (h as f64 * scale).round().max(1.0) as u32;
    let resized =
        image::imageops::resize(&rgba, new_w, new_h, image::imageops::FilterType::Nearest);
    render_rgba_image(&resized, new_w, new_h)
}

/// Convert an RGBA pixel buffer to half-block `Line`s.
fn render_rgba_image(rgba: &image::RgbaImage, width: u32, height: u32) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut y = 0u32;
    while y < height {
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(width as usize);
        for x in 0..width {
            let top = rgba.get_pixel(x, y);
            let bottom = if y + 1 < height {
                rgba.get_pixel(x, y + 1)
            } else {
                // No bottom row — use transparent (show as space bg)
                &image::Rgba([0, 0, 0, 0])
            };

            let top_color = rgba_to_color(top);
            let bottom_color = rgba_to_color(bottom);

            let ch = if top[3] == 0 && bottom[3] == 0 {
                ' '
            } else if top[3] == 0 {
                '▄' // bottom only → bg is bottom, fg transparent
            } else {
                // top opaque (bottom transparent OR both opaque): fg is top
                '▀'
            };

            spans.push(Span::styled(
                ch.to_string(),
                Style::default().fg(top_color).bg(bottom_color),
            ));
        }
        lines.push(Line::from(spans));
        y += 2;
    }
    lines
}

fn rgba_to_color(px: &image::Rgba<u8>) -> Color {
    Color::Rgb(px[0], px[1], px[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    #[test]
    fn decode_data_uri_extracts_bytes() {
        // base64 of "hello"
        let uri = "data:image/png;base64,aGVsbG8=";
        let bytes = decode_data_uri(uri).unwrap();
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn decode_data_uri_returns_none_for_http() {
        assert!(decode_data_uri("https://example.com/image.png").is_none());
    }

    #[test]
    fn decode_data_uri_returns_none_for_invalid_base64() {
        assert!(decode_data_uri("data:image/png;base64,!!!invalid!!!").is_none());
    }

    #[test]
    fn decode_data_uri_returns_none_for_non_base64() {
        assert!(decode_data_uri("data:image/png,rawdata").is_none());
    }

    #[test]
    fn render_image_halfblock_produces_lines() {
        // Create a small 2x2 red image
        let img = image::RgbaImage::from_raw(
            2,
            2,
            vec![
                255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
            ],
        )
        .unwrap();
        let mut buf = Vec::new();
        image::codecs::png::PngEncoder::new(&mut buf)
            .write_image(img.as_raw(), 2, 2, image::ExtendedColorType::Rgba8)
            .unwrap();
        let lines = render_image_halfblock(&buf, 80);
        assert!(
            !lines.is_empty(),
            "should produce at least 1 line for 2px height"
        );
        // 2 pixel rows → 1 half-block line
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn render_image_halfblock_empty_on_garbage() {
        let lines = render_image_halfblock(b"not an image", 80);
        assert!(lines.is_empty());
    }

    #[test]
    fn render_rgba_image_pairs_two_rows() {
        let img = image::RgbaImage::from_raw(2, 4, vec![255; 32]).unwrap();
        let lines = render_rgba_image(&img, 2, 4);
        // 4 pixel rows / 2 = 2 half-block lines
        assert_eq!(lines.len(), 2);
        // Each line should have 2 spans (2px wide)
        assert_eq!(lines[0].spans.len(), 2);
    }
}
