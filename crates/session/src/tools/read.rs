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
        "Reads a UTF-8 text file from the filesystem. Returns up to 200 lines by default (max 1000 per call). Appends a metadata footer (total_lines, offset, lines_read); a notice is shown only when the token limit truncates the requested range."
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

        // Edge case: the very first line at the requested offset exceeded the
        // token budget on its own, so the loop broke at i==0 before `actual_end`
        // was advanced. Left as-is, `lines_read` would be 0 and the truncation
        // notice would tell the model to re-read at the SAME offset
        // (`actual_end + 1 == start + 1 == offset`) — an infinite retry loop.
        // Skip the oversized line so the next read makes progress.
        let first_line_oversized = actual_end == start && start < requested_end;
        if first_line_oversized {
            actual_end = start + 1;
        }

        let lines_read = actual_end - start;
        let token_capped = actual_end < requested_end;

        if out.is_empty() {
            out.push_str("(empty)\n");
        }

        out.push('\n');
        if token_capped {
            let next = actual_end + 1;
            if first_line_oversized {
                out.push_str(&format!(
                    "[INCOMPLETE READ] line {} exceeded the token limit and was skipped; re-read with offset={} to continue.\n",
                    start + 1, next
                ));
            } else {
                out.push_str(&format!(
                    "[INCOMPLETE READ] output truncated at token limit; re-read with offset={} to continue.\n",
                    next
                ));
            }
        }
        out.push_str("--- metadata ---\n");
        out.push_str(&format!("total_lines: {}\n", total_lines));
        out.push_str(&format!("offset: {}\n", start + 1));
        out.push_str(&format!("lines_read: {}\n", lines_read));

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

    async fn run_read(
        tool: &super::ReadTool,
        ctx: &ToolContext,
        input: serde_json::Value,
    ) -> String {
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
        assert!(!out.contains("[INCOMPLETE READ]"));
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
        let content_end = out.find("\n\n").unwrap();
        assert!(opencoder_llm::estimate(&out[..content_end]) <= 5000);
        assert!(out.contains("[INCOMPLETE READ]"));
        assert!(out.contains("offset="));
    }

    #[tokio::test]
    async fn test_oversized_first_line_skipped_no_loop() {
        // Regression (Bug 1): when the FIRST line at the requested offset
        // alone exceeds the token budget, the read must still ADVANCE past it
        // (lines_read=1, re-read offset = offset + 1) instead of telling the
        // model to re-read the SAME offset, which caused an infinite loop.
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("f.txt")).unwrap();
        // Line 1 is a single oversized word (>5000 tokens => >20000 chars).
        writeln!(f, "{}", "x".repeat(25_000)).unwrap();
        for i in 0..5 {
            writeln!(f, "small {}", i).unwrap();
        }
        let ctx = ctx_for(&dir);
        let tool = super::ReadTool;
        let out = run_read(&tool, &ctx, json!({ "path": "f.txt", "offset": 1 })).await;
        let meta = parse_metadata(&out);
        // The oversized line is skipped but counted as read so progress is made.
        assert_eq!(meta.get("offset"), Some(&"1"));
        assert_eq!(meta.get("lines_read"), Some(&"1"), "{}", out);
        // The notice must point PAST the oversized line (offset=2), never back
        // at the same offset=1 — that was the infinite-loop bug.
        let notice = out
            .lines()
            .find(|l| l.starts_with("[INCOMPLETE READ]"))
            .expect("truncation notice present");
        assert!(
            notice.contains("exceeded the token limit and was skipped"),
            "{}",
            notice
        );
        assert!(notice.contains("offset=2"), "{}", notice);
        assert!(
            !notice.contains("offset=1"),
            "must not repeat the same offset (infinite loop): {}",
            notice
        );
    }

    #[tokio::test]
    async fn test_oversized_line_mid_read_keeps_wording() {
        // When truncation happens AFTER at least one line was emitted (i.e. not
        // the first-line-oversized case), the original "output truncated"
        // wording must be preserved and the read still advances.
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("f.txt")).unwrap();
        // Line 1 is small (emitted), line 2 alone blows the budget.
        writeln!(f, "small first line").unwrap();
        writeln!(f, "{}", "y".repeat(25_000)).unwrap();
        for i in 0..3 {
            writeln!(f, "tail {}", i).unwrap();
        }
        let ctx = ctx_for(&dir);
        let tool = super::ReadTool;
        let out = run_read(&tool, &ctx, json!({ "path": "f.txt", "offset": 1 })).await;
        let meta = parse_metadata(&out);
        assert_eq!(meta.get("lines_read"), Some(&"1"), "{}", out);
        let notice = out
            .lines()
            .find(|l| l.starts_with("[INCOMPLETE READ]"))
            .expect("truncation notice present");
        assert!(
            notice.contains("output truncated at token limit"),
            "non-first-line truncation keeps original wording: {}",
            notice
        );
        assert!(notice.contains("offset=2"), "{}", notice);
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
        // offset read with no token-cap must not emit a redundant notice.
        assert!(!out.contains("[INCOMPLETE READ]"));
    }

    #[tokio::test]
    async fn test_offset_remaining_content_no_notice() {
        // Reading a slice that stops at the line limit (not token cap) while
        // content remains afterwards: this is expected pagination, NOT an
        // incomplete read, so no notice should be emitted.
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("f.txt")).unwrap();
        for i in 0..500 {
            writeln!(f, "line {}", i).unwrap();
        }
        let ctx = ctx_for(&dir);
        let tool = super::ReadTool;
        let out = run_read(
            &tool,
            &ctx,
            json!({ "path": "f.txt", "offset": 51, "limit": 100 }),
        )
        .await;
        let meta = parse_metadata(&out);
        assert_eq!(meta.get("offset"), Some(&"51"));
        assert_eq!(meta.get("lines_read"), Some(&"100"));
        // 400 lines still remain, but this is a normal line-limit stop.
        assert!(!out.contains("[INCOMPLETE READ]"));
    }
}
