use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, Tool, ToolContext, ToolOutput};
use opencoder_llm::estimate as estimate_tokens;
use serde_json::Value;

/// Default number of lines to read when no `limit` is specified.
const DEFAULT_LIMIT: usize = 200;
/// Maximum number of lines that can be read in a single call.
const MAX_LIMIT: usize = 1000;
/// Maximum estimated tokens allowed in a single read response.
const MAX_TOKENS: usize = 5000;

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
        "Reads a UTF-8 text file from the filesystem. Returns up to 200 lines by default (max 1000 per call). Appends a metadata footer (total_lines, offset, lines_read). When the file is not fully read, an [INCOMPLETE READ] hint with the next offset is appended."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "path".into(),
            json::prop_str("Path to the file to read, relative to the working directory."),
        );
        props.insert("offset".into(), serde_json::json!({ "type": "integer", "description": "Starting 1-based line number (optional, default 1)." }));
        props.insert("limit".into(), serde_json::json!({ "type": "integer", "description": "Max number of lines to read (optional, default 200, max 1000)." }));
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
            .map(|n| (n as usize).min(MAX_LIMIT))
            .unwrap_or(DEFAULT_LIMIT)
            .min(MAX_LIMIT);

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();
        let start = (offset - 1).min(total_lines);
        let requested_end = (start + limit).min(total_lines);

        let mut out = String::new();
        let mut actual_end = start;
        for (i, line) in lines[start..requested_end].iter().enumerate() {
            let candidate = format!("{:>5}: {}\n", start + i + 1, expand_tabs(line));
            let mut trial = out.clone();
            trial.push_str(&candidate);
            if estimate_tokens(&trial) > MAX_TOKENS {
                break;
            }
            out = trial;
            actual_end = start + i + 1;
        }

        let lines_read = actual_end - start;

        if out.is_empty() {
            out.push_str("(empty)\n");
        }

        out.push('\n');
        out.push_str("--- metadata ---\n");
        out.push_str(&format!("total_lines: {}\n", total_lines));
        out.push_str(&format!("offset: {}\n", start + 1));
        out.push_str(&format!("lines_read: {}\n", lines_read));

        if actual_end < total_lines {
            // Determine the reason for stopping when the file wasn't fully read.
            // `stopped_by_tokens` is true when we broke out of the loop before
            // reaching the requested line count because of the token budget.
            let stopped_by_tokens = actual_end < requested_end;
            // `stopped_by_line_limit` is true when we read all requested lines but
            // there are still more lines in the file.
            let stopped_by_line_limit = actual_end == requested_end && requested_end < total_lines;
            let reason = match (stopped_by_tokens, stopped_by_line_limit) {
                (true, true) => "both token and line limits reached",
                (true, false) => "token limit (5000) reached",
                (false, true) => "line limit reached",
                _ => "file ended unexpectedly",
            };
            let next_offset = actual_end + 1;
            out.push_str(&format!(
                "[INCOMPLETE READ] The file has not been fully read. Stopped because: {}. To continue reading, call read again with offset={}.\n",
                reason, next_offset
            ));
        }

        Ok(ToolOutput::ok(out))
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

    // --- pagination / metadata tests ---

    use opencoder_core::{Tool, ToolContext};
    use serde_json::json;
    use std::io::Write;

    fn ctx_for(dir: &tempfile::TempDir) -> ToolContext {
        ToolContext {
            session_id: "test".into(),
            message_id: "test".into(),
            agent: "explore".into(),
            working_dir: dir.path().to_path_buf(),
            max_output: 4096,
            proxy: None,
        }
    }

    async fn run_read(tool: &super::ReadTool, ctx: &ToolContext, input: serde_json::Value) -> String {
        tool.execute(input, ctx).await.unwrap().content
    }

    fn parse_metadata(out: &str) -> std::collections::HashMap<&str, &str> {
        let mut map = std::collections::HashMap::new();
        for line in out.lines().rev() {
            if line == "--- metadata ---" {
                break;
            }
            if let Some((k, v)) = line.split_once(": ") {
                map.insert(k.trim(), v.trim());
            }
        }
        map
    }

    #[tokio::test]
    async fn test_default_limit_200() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("f.txt")).unwrap();
        for i in 0..500 {
            writeln!(f, "line {}", i).unwrap();
        }
        let ctx = ctx_for(&dir);
        let tool = super::ReadTool;
        let out = run_read(&tool, &ctx, json!({ "path": "f.txt" })).await;
        let meta = parse_metadata(&out);
        assert_eq!(meta.get("total_lines"), Some(&"500"));
        assert_eq!(meta.get("lines_read"), Some(&"200"));
    }

    #[tokio::test]
    async fn test_max_limit_1000() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("f.txt")).unwrap();
        for i in 0..2000 {
            writeln!(f, "line {}", i).unwrap();
        }
        let ctx = ctx_for(&dir);
        let tool = super::ReadTool;
        let out = run_read(&tool, &ctx, json!({ "path": "f.txt", "limit": 5000 })).await;
        let meta = parse_metadata(&out);
        assert_eq!(meta.get("lines_read"), Some(&"1000"));
    }

    #[tokio::test]
    async fn test_token_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("f.txt")).unwrap();
        let long = "x".repeat(200);
        for _ in 0..500 {
            writeln!(f, "{}", long).unwrap();
        }
        let ctx = ctx_for(&dir);
        let tool = super::ReadTool;
        let out = run_read(&tool, &ctx, json!({ "path": "f.txt", "limit": 1000 })).await;
        let meta = parse_metadata(&out);
        // Must stop before reaching 1000 lines because of token cap.
        let lines_read: usize = meta.get("lines_read").unwrap().parse().unwrap();
        assert!(lines_read < 1000, "lines_read={}", lines_read);
        let content_end = out.find("\n\n--- metadata ---").unwrap();
        assert!(opencoder_llm::estimate(&out[..content_end]) <= 5000);
    }

    #[tokio::test]
    async fn test_metadata_no_more() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("f.txt")).unwrap();
        for i in 0..10 {
            writeln!(f, "line {}", i).unwrap();
        }
        let ctx = ctx_for(&dir);
        let tool = super::ReadTool;
        let out = run_read(&tool, &ctx, json!({ "path": "f.txt" })).await;
        let meta = parse_metadata(&out);
        assert_eq!(meta.get("lines_read"), Some(&"10"));
        assert_eq!(meta.get("total_lines"), Some(&"10"));
    }

    #[tokio::test]
    async fn test_offset_pagination() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("f.txt")).unwrap();
        for i in 0..300 {
            writeln!(f, "line {}", i).unwrap();
        }
        let ctx = ctx_for(&dir);
        let tool = super::ReadTool;
        let out = run_read(&tool, &ctx, json!({ "path": "f.txt", "offset": 201 })).await;
        let meta = parse_metadata(&out);
        assert_eq!(meta.get("offset"), Some(&"201"));
        // first content line should be line 201
        let first_line = out.lines().next().unwrap();
        assert!(first_line.starts_with("  201:"), "{}", first_line);
    }
}
