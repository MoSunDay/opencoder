//! Half-block image rendering for the TUI chat transcript. Decodes image
//! bytes, scales to terminal width, and maps each 2-pixel-row to a single
//! character cell using Unicode half-block characters with foreground/background Color::Rgb.
//!
//! For data URIs (data:image/<fmt>;base64,...) the raw bytes are decoded
//! in-process. HTTP(S) URLs are fetched asynchronously via [`fetch_image_bytes`]
//! (used by the async replay path); the synchronous [`build_image_block`] falls
//! back to a placeholder for remote URLs.
//!
//! Image width adapts to the live terminal size via [`terminal_image_width`],
//! and downscaling uses [`image::imageops::FilterType::Triangle`] for smoother
//! quality than nearest-neighbour.

use base64::Engine;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use std::collections::HashMap;
use std::sync::OnceLock;

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

/// Indent applied to image lines in `flatten_with` (4 spaces) plus the
/// rounded-block border (2) and the `text_w` subtraction (1) in `render.rs`.
const IMAGE_WIDTH_OVERHEAD: u16 = 8;

/// Determine the best width (in character cells) for rendering inline images.
/// Queries the live terminal size and subtracts overhead (indent + borders)
/// so images fit without overflow. Falls back to 120 when the terminal size
/// cannot be queried (e.g. piped output or headless tests).
pub fn terminal_image_width() -> u16 {
    crossterm::terminal::size()
        .map(|(cols, _)| cols.saturating_sub(IMAGE_WIDTH_OVERHEAD).max(20))
        .unwrap_or(120)
}

/// Shared HTTP client with a 10-second timeout. Memoised via `OnceLock` so
/// repeated fetches reuse the same connection pool.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Fetch image bytes from an HTTP(S) URL. Returns `None` on network or
/// decode errors.
async fn fetch_http_bytes(url: &str) -> Option<Vec<u8>> {
    http_client()
        .get(url)
        .send()
        .await
        .ok()?
        .bytes()
        .await
        .ok()
        .map(|b| b.to_vec())
}

/// Fetch image bytes from any URL type: data URI (decoded in-process) or
/// HTTP(S) (fetched with a timeout). Returns `None` on failure.
///
/// Used by the async replay path (`replay_into_chat`) to pre-fetch remote
/// images so they render during synchronous message replay.
pub async fn fetch_image_bytes(url: &str) -> Option<Vec<u8>> {
    if url.starts_with("data:") {
        return decode_data_uri(url);
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return fetch_http_bytes(url).await;
    }
    None
}

/// Build a `(filename, rendered_lines)` pair suitable for a `ChatBlock::Image`
/// from an image URL (data URI or http URL). Data URIs are decoded and
/// rendered as half-block art; remote URLs yield a placeholder (empty lines)
/// since we cannot fetch synchronously.
pub fn build_image_block(url: &str) -> (String, Vec<Line<'static>>) {
    let filename =
        crate::terminal_text::sanitize_single_line(&crate::image_util::extract_filename(url))
            .into_owned();
    let width = terminal_image_width();
    let rendered = decode_data_uri(url)
        .map(|bytes| render_image_halfblock(&bytes, width))
        .unwrap_or_default();
    (filename, rendered)
}

/// Render image half-block lines from a URL, checking a prefetched-bytes map
/// first (populated by async pre-fetch in `replay_into_chat`), then falling
/// back to synchronous data-URI decoding. The width adapts to the terminal.
pub fn render_image_from_url(
    url: &str,
    prefetched: &HashMap<String, Vec<u8>>,
) -> Vec<Line<'static>> {
    let width = terminal_image_width();
    if let Some(bytes) = prefetched.get(url) {
        return render_image_halfblock(bytes, width);
    }
    decode_data_uri(url)
        .map(|bytes| render_image_halfblock(&bytes, width))
        .unwrap_or_default()
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
        image::imageops::resize(&rgba, new_w, new_h, image::imageops::FilterType::Triangle);
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

#[cfg(test)]
mod width_tests {
    use super::*;
    use image::ImageEncoder;

    /// `terminal_image_width` always returns a value >= 20 (the minimum).
    /// In a headless CI environment it falls back to 120.
    #[test]
    fn terminal_image_width_returns_at_least_minimum() {
        let w = terminal_image_width();
        assert!(w >= 20, "width {w} below minimum of 20");
    }

    /// `fetch_image_bytes` works synchronously for data URIs.
    #[tokio::test]
    async fn fetch_image_bytes_decodes_data_uri() {
        let uri = "data:image/png;base64,aGVsbG8=";
        let bytes = fetch_image_bytes(uri).await;
        assert_eq!(bytes, Some(b"hello".to_vec()));
    }

    /// `fetch_image_bytes` returns None for unrecognized schemes.
    #[tokio::test]
    async fn fetch_image_bytes_returns_none_for_unknown_scheme() {
        assert!(fetch_image_bytes("ftp://example.com/img.png")
            .await
            .is_none());
        assert!(fetch_image_bytes("not-a-url").await.is_none());
    }

    /// `render_image_from_url` uses prefetched bytes when available.
    #[test]
    fn render_image_from_url_uses_prefetched_bytes() {
        // Build a small valid PNG
        let img = image::RgbaImage::from_raw(
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255,
            ],
        )
        .unwrap();
        let mut buf = Vec::new();
        image::codecs::png::PngEncoder::new(&mut buf)
            .write_image(img.as_raw(), 2, 2, image::ExtendedColorType::Rgba8)
            .unwrap();

        let url = "https://example.com/test.png";
        let mut map = HashMap::new();
        map.insert(url.to_string(), buf.clone());

        let lines = render_image_from_url(url, &map);
        assert!(!lines.is_empty(), "prefetched image should render");
        assert_eq!(lines.len(), 1, "2px tall image = 1 half-block line");
    }

    /// `render_image_from_url` falls back to data-URI decoding when the URL
    /// is not in the prefetched map.
    #[test]
    fn render_image_from_url_falls_back_to_data_uri() {
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
        let uri = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&buf)
        );

        let empty_map = HashMap::new();
        let lines = render_image_from_url(&uri, &empty_map);
        assert!(!lines.is_empty(), "data URI should render without prefetch");
    }

    /// `render_image_from_url` returns empty Vec for unknown URLs without
    /// prefetch data.
    #[test]
    fn render_image_from_url_empty_for_missing_http() {
        let empty_map = HashMap::new();
        let lines = render_image_from_url("https://example.com/missing.png", &empty_map);
        assert!(
            lines.is_empty(),
            "missing HTTP URL without prefetch should yield empty"
        );
    }

    /// Triangle filter produces smooth output for a large image (smoke test
    /// ensuring the filter type change doesn't panic or produce zero output).
    #[test]
    fn triangle_filter_renders_large_image() {
        let img = image::RgbaImage::from_raw(400, 200, vec![128; 400 * 200 * 4]).unwrap();
        let mut buf = Vec::new();
        image::codecs::png::PngEncoder::new(&mut buf)
            .write_image(img.as_raw(), 400, 200, image::ExtendedColorType::Rgba8)
            .unwrap();
        // Render at narrow width to force downscaling
        let lines = render_image_halfblock(&buf, 60);
        assert!(!lines.is_empty(), "large image should produce output");
        // 200px / 2 = 100 half-block lines at full size; downscaling keeps
        // the aspect ratio so height shrinks proportionally.
        let expected_h = ((200.0_f64 * 60.0_f64 / 400.0_f64 / 2.0_f64).round()) as usize;
        assert_eq!(lines.len(), expected_h, "downscaled height should match");
    }
}
