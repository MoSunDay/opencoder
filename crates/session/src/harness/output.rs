use serde_json::Value;

pub fn extract_output_json_from(text: &str) -> Option<Value> {
    // When a fence exists but its body fails to parse, the bare-JSON scan
    // runs on the text AFTER the fence body: an unclosed `{` inside the dead
    // fence must not swallow the real object that follows it.
    let mut bare_scan = text;
    if let Some(start) = text.rfind("```json") {
        let after = &text[start + "```json".len()..];
        let (body, rest) = match after.find("```") {
            Some(end) => (&after[..end], &after[end + "```".len()..]),
            None => (after, ""),
        };
        bare_scan = rest;
        if let Ok(v) = serde_json::from_str::<Value>(body.trim()) {
            return Some(v);
        }
    }
    if let Some(v) = extract_tail_bare_json(bare_scan) {
        return Some(v);
    }
    serde_json::from_str::<Value>(text.trim()).ok()
}

/// How much of the reply tail the bare-JSON fallback scans. Step replies are
/// narration-first, so the final structured object always sits near the end.
const BARE_JSON_TAIL_LIMIT: usize = 8 * 1024;

/// Find every balanced top-level `{...}` span in the tail of `text` and parse
/// the last one that yields valid JSON. Braces inside JSON strings never
/// count (string-aware scan). Byte-level scanning is safe: `{`, `}`, `"`,
/// `\` are ASCII and never occur inside a multi-byte UTF-8 sequence, so span
/// slicing always lands on char boundaries.
fn extract_tail_bare_json(text: &str) -> Option<Value> {
    let start = if text.len() <= BARE_JSON_TAIL_LIMIT {
        0
    } else {
        let mut cut = text.len() - BARE_JSON_TAIL_LIMIT;
        while !text.is_char_boundary(cut) {
            cut += 1;
        }
        cut
    };
    let bytes = &text.as_bytes()[start..];
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut depth = 0usize;
    let mut open_at: Option<usize> = None;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &byte) in bytes.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    open_at = Some(i);
                }
                depth += 1;
            }
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    spans.push((open_at.take().unwrap_or(i), i));
                }
            }
            _ => {}
        }
    }
    spans
        .iter()
        .rev()
        .find_map(|(from, to)| serde_json::from_slice(&bytes[*from..=*to]).ok())
}

#[cfg(test)]
mod tests {
    #[test]
    fn structured_answer_accepts_fence_and_tail_and_rejects_invalid_json() {
        assert_eq!(
            super::extract_output_json_from("answer\n{\"ok\":true}"),
            Some(serde_json::json!({"ok":true}))
        );
        assert_eq!(
            super::extract_output_json_from("```json\n{\"ok\":true}\n```"),
            Some(serde_json::json!({"ok":true}))
        );
        assert!(super::extract_output_json_from("answer {broken").is_none());
    }
}
