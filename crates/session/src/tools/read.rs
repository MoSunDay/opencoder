use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, Tool, ToolContext, ToolOutput};
use serde_json::Value;

/// Expand tab characters to spaces, advancing to the next 8-column tab stop.
/// ratatui counts a tab as 0 columns but a terminal expands it to the next
/// multiple of 8, which shifted file content past the line-number gutter on
/// macOS. Expanding here keeps the gutter visually aligned with content.
fn expand_tabs(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut col = 0usize;
    for ch in line.chars() {
        if ch == '\t' {
            let spaces = 8 - (col % 8);
            out.extend(std::iter::repeat_n(' ', spaces));
            col += spaces;
        } else {
            out.push(ch);
            col += 1;
        }
    }
    out
}

pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }
    fn description(&self) -> &str {
        "Reads a UTF-8 text file from the filesystem. Supports optional line offset and limit. Returns the file content."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "path".into(),
            json::prop_str("Path to the file to read, relative to the working directory."),
        );
        props.insert("offset".into(), serde_json::json!({ "type": "integer", "description": "Starting 1-based line number (optional)." }));
        props.insert("limit".into(), serde_json::json!({ "type": "integer", "description": "Max number of lines to read (optional)." }));
        json::object_schema(Value::Object(props), &["path"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let full = resolve(ctx, path);
        let content = match tokio::fs::read_to_string(&full).await {
            Ok(c) => c,
            Err(e) => return Ok(ToolOutput::err(format!("read {}: {e}", full.display()))),
        };
        let offset = input
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            .max(1) as usize;
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let lines: Vec<&str> = content.lines().collect();
        let start = (offset - 1).min(lines.len());
        let end = match limit {
            Some(n) => (start + n).min(lines.len()),
            None => lines.len(),
        };
        let mut out = String::new();
        for (i, line) in lines[start..end].iter().enumerate() {
            out.push_str(&format!("{:>5}: {}\n", start + i + 1, expand_tabs(line)));
        }
        if out.is_empty() {
            out.push_str("(empty)");
        }
        Ok(opencoder_core::tool::truncate_output(out, ctx.max_output))
    }
}

pub(crate) fn resolve(ctx: &ToolContext, path: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        ctx.working_dir.join(p)
    }
}

#[cfg(test)]
mod tests {
    use super::expand_tabs;

    #[test]
    fn expand_leading_tab() {
        // A single leading tab -> 8 spaces.
        assert_eq!(expand_tabs("\tcode"), "        code");
    }

    #[test]
    fn expand_mid_line_tab_advances_to_next_stop() {
        // "ab" (2 cols) then tab -> next stop at 8, so 6 spaces.
        assert_eq!(expand_tabs("ab\tcd"), "ab      cd");
    }

    #[test]
    fn expand_consecutive_tabs() {
        // tab (0->8), tab (8->16): 16 spaces total.
        assert_eq!(expand_tabs("\t\tend"), "                end");
    }

    #[test]
    fn no_tab_returns_unchanged() {
        assert_eq!(expand_tabs("plain text"), "plain text");
    }

    #[test]
    fn tab_at_eighth_column_adds_eight_spaces() {
        // Exactly 8 cols already -> tab goes to the NEXT multiple of 8 (16).
        assert_eq!(expand_tabs("12345678\tx"), "12345678        x");
    }

    #[test]
    fn empty_string_unchanged() {
        assert_eq!(expand_tabs(""), "");
    }
}
