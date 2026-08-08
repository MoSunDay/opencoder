//! Chunked image upload frame protocol + pure accumulator.
//!
//! Terminals (tmux/SSH) may truncate huge single pastes, so
//! `scripts/img2uri.sh --chunk` emits an image as small self-delimiting text
//! frames the user pastes one-by-one or all at once; the TUI reassembles
//! them into one `data:image/<fmt>;base64,…` URI.
//!
//! Accumulation is string concatenation ONLY — never decodes base64 — so
//! feeding frames cannot block the event loop. Frames are self-delimiting
//! and order-independent: chunks may arrive in any order and duplicate
//! re-pastes are idempotent (same `seq` overwrites).
//!
//! Protocol (one frame per line), `seq` is 0-based:
//!
//! ```text
//! ocimg begin <id> <fmt> <total>
//! ocimg chunk <id> <seq> <base64…>
//! ocimg end <id>
//! ```

use std::collections::{BTreeMap, HashMap};

/// In-flight assemblies older than this are considered abandoned.
pub const STALE_TIMEOUT_MS: u64 = 5 * 60 * 1000;

/// Milliseconds since the UNIX epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// One parsed protocol frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Begin {
        id: String,
        fmt: String,
        total: usize,
    },
    Chunk {
        id: String,
        seq: usize,
        data: String,
    },
    End {
        id: String,
    },
}

/// Parse one line into a [`Frame`]. Returns None for anything that is not
/// exactly a well-formed `ocimg` frame; extra trailing tokens are rejected.
pub fn parse_frame(line: &str) -> Option<Frame> {
    let mut toks = line.split_whitespace();
    match (toks.next(), toks.next()) {
        (Some("ocimg"), Some("begin")) => {
            let id = toks.next()?.to_string();
            let fmt = toks.next()?.to_string();
            let total = toks.next()?.parse::<usize>().ok()?;
            (toks.next().is_none()).then_some(Frame::Begin { id, fmt, total })
        }
        (Some("ocimg"), Some("chunk")) => {
            let id = toks.next()?.to_string();
            let seq = toks.next()?.parse::<usize>().ok()?;
            let data = toks.next()?.to_string();
            (toks.next().is_none()).then_some(Frame::Chunk { id, seq, data })
        }
        (Some("ocimg"), Some("end")) => {
            let id = toks.next()?.to_string();
            (toks.next().is_none()).then_some(Frame::End { id })
        }
        _ => None,
    }
}

/// Map a format token to a MIME type, case-insensitively.
fn mime_from_fmt(fmt: &str) -> Option<&'static str> {
    match fmt.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

/// Canonical file extension for a format token; unknown falls back to "png".
fn ext_from_fmt(fmt: &str) -> &'static str {
    match fmt.to_ascii_lowercase().as_str() {
        "png" => "png",
        "jpg" | "jpeg" => "jpeg",
        "gif" => "gif",
        "webp" => "webp",
        "bmp" => "bmp",
        _ => "png",
    }
}

/// Result of feeding one line into an [`Assembly`]: `NotFrame` (not an
/// `ocimg` line), `Pending` (accepted, awaiting more chunks), `Complete`
/// (assembly removed, full data URI returned) or `Warn` (malformed or
/// inconsistent frame; nothing assembled).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedOutcome {
    NotFrame,
    Pending,
    Complete {
        uri: String,
        filename: String,
        chunks: usize,
    },
    Warn {
        message: String,
    },
}

/// One in-flight image assembly.
#[derive(Debug, Clone)]
struct Entry {
    fmt: String,
    total: usize,
    chunks: BTreeMap<usize, String>,
    started_ms: u64,
}

/// Pure accumulator for chunked image frames. Feed lines one at a time;
/// completed assemblies are returned as data URIs and removed from state.
#[derive(Debug, Default, Clone)]
pub struct Assembly {
    entries: HashMap<String, Entry>,
}

impl Assembly {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Number of in-flight (started but not yet completed) assemblies.
    pub fn in_flight(&self) -> usize {
        self.entries.len()
    }

    /// Feed one pasted line. See [`FeedOutcome`] for result semantics.
    pub fn feed_line(&mut self, line: &str, now_ms: u64) -> FeedOutcome {
        let frame = match parse_frame(line) {
            Some(f) => f,
            None => return FeedOutcome::NotFrame,
        };
        match frame {
            Frame::Begin { id, fmt, total } => {
                if mime_from_fmt(&fmt).is_none() {
                    return FeedOutcome::Warn {
                        message: format!("[img] unknown image format '{fmt}' in frame for '{id}'"),
                    };
                }
                if total == 0 {
                    return FeedOutcome::Warn {
                        message: format!("[img] zero chunk count in begin frame for '{id}'"),
                    };
                }
                let entry = Entry {
                    fmt,
                    total,
                    chunks: BTreeMap::new(),
                    started_ms: now_ms,
                };
                if self.entries.insert(id.clone(), entry).is_some() {
                    FeedOutcome::Warn {
                        message: format!("[img] duplicate begin for '{id}' — restarted"),
                    }
                } else {
                    FeedOutcome::Pending
                }
            }
            Frame::Chunk { id, seq, data } => {
                let Some(entry) = self.entries.get_mut(&id) else {
                    return FeedOutcome::Warn {
                        message: format!("[img] chunk for unknown id '{id}' — dropped"),
                    };
                };
                if seq >= entry.total {
                    return FeedOutcome::Warn {
                        message: format!(
                            "[img] chunk seq {seq} out of range for '{id}' (total {})",
                            entry.total
                        ),
                    };
                }
                // Duplicate seq overwrites — idempotent re-paste.
                entry.chunks.insert(seq, data);
                FeedOutcome::Pending
            }
            Frame::End { id } => {
                let Some(entry) = self.entries.remove(&id) else {
                    return FeedOutcome::Warn {
                        message: format!("[img] end for unknown id '{id}' — dropped"),
                    };
                };
                let got = entry.chunks.len();
                let ordered =
                    got == entry.total && entry.chunks.keys().enumerate().all(|(i, k)| *k == i);
                if !ordered {
                    return FeedOutcome::Warn {
                        message: format!(
                            "[img] '{id}' ended with {got}/{} chunks — dropped",
                            entry.total
                        ),
                    };
                }
                let payload: String = entry.chunks.into_values().collect();
                let mime = mime_from_fmt(&entry.fmt).unwrap_or("image/png");
                let ext = ext_from_fmt(&entry.fmt);
                FeedOutcome::Complete {
                    uri: format!("data:{mime};base64,{payload}"),
                    filename: format!("pasted.{ext}"),
                    chunks: entry.total,
                }
            }
        }
    }

    /// Remove entries older than `timeout_ms`; returns `(id, chunks, total)`.
    pub fn drain_stale(&mut self, now_ms: u64, timeout_ms: u64) -> Vec<(String, usize, usize)> {
        self.entries
            .extract_if(|_, e| now_ms.saturating_sub(e.started_ms) >= timeout_ms)
            .map(|(id, e)| (id, e.chunks.len(), e.total))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Paste classification helpers (used by `app_loop::route_paste`)
// ---------------------------------------------------------------------------

/// If `s` is a `data:image/<fmt>;base64,...` URI, return the canonical
/// filename (`pasted.<ext>`). Non-image data URIs and non-data-URI strings
/// return None.
pub fn image_data_uri_filename(s: &str) -> Option<String> {
    let rest = s.strip_prefix("data:")?;
    let semi = rest.find(';')?;
    let mime = &rest[..semi];
    let (type_, subtype) = mime.split_once('/')?;
    if type_ != "image" || mime_from_fmt(subtype).is_none() {
        return None;
    }
    if !rest[semi..].contains("base64") {
        return None;
    }
    Some(format!("pasted.{}", ext_from_fmt(subtype)))
}

/// If `s` is an HTTP(S) URL whose last path segment carries an image
/// extension, return that segment (the display filename). Query strings
/// and fragments are tolerated: the extension is checked on the path
/// portion only. Returns None for non-URLs or URLs without an image suffix.
pub fn image_url_filename(s: &str) -> Option<String> {
    if !s.starts_with("http://") && !s.starts_with("https://") {
        return None;
    }
    let path = s.split(['?', '#']).next()?;
    let last = path.rsplit('/').next()?;
    let (_, ext) = last.rsplit_once('.')?;
    if mime_from_fmt(ext).is_some() {
        Some(last.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
