//! Ripgrep-library code search.
//!
//! Built on the ripgrep engine crates (`grep-regex` + `grep-searcher`) and the
//! `ignore` walker, so the user needs no `rg` binary installed: the matching is
//! the same in-process engine, and `.gitignore` / `.ignore` / hidden files are
//! honoured exactly as ripgrep does by default.

use std::io;
use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{Searcher, SearcherBuilder, Sink, SinkMatch};
use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use opencoder_core::{json, tool::truncate_output, Tool, ToolContext, ToolOutput};
use serde_json::Value;

/// Maximum number of matching lines returned before the search short-circuits.
const MAX_MATCHES: usize = 1000;

pub struct SearchTool;

#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }
    fn description(&self) -> &str {
        "Searches file contents for a regex and returns matching lines as `path:line: content`. \
         Respects .gitignore and .ignore by default and skips hidden/binary files. \
         Provide `include` to filter by file name (e.g. \"*.rs\"). Powered by the ripgrep engine."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "pattern".into(),
            json::prop_str("Regular expression to search for."),
        );
        props.insert(
            "path".into(),
            json::prop_str("Optional directory or file to search in (default: working directory)."),
        );
        props.insert(
            "include".into(),
            json::prop_str("Optional glob filter for file names, e.g. \"*.rs\"."),
        );
        json::object_schema(Value::Object(props), &["pattern"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if pattern.is_empty() {
            return Ok(ToolOutput::err("search requires a non-empty 'pattern'"));
        }
        let path_str = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let include = input
            .get("include")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let base = match &path_str {
            Some(p) => super::read::resolve(ctx, p),
            None => ctx.working_dir.clone(),
        };
        let max_output = ctx.max_output;

        let out = tokio::task::spawn_blocking(move || -> ToolOutput {
            let matcher = match RegexMatcherBuilder::new().build(&pattern) {
                Ok(m) => m,
                Err(e) => return ToolOutput::err(format!("invalid regex: {e}")),
            };

            let mut collector = Collector {
                results: Vec::new(),
                rel: String::new(),
                max: MAX_MATCHES,
            };
            let mut searcher = SearcherBuilder::new().line_number(true).build();

            if base.is_file() {
                collector.rel = path_str
                    .clone()
                    .unwrap_or_else(|| base.display().to_string());
                let _ = searcher.search_path(&matcher, &base, &mut collector);
            } else {
                let mut wb = WalkBuilder::new(&base);
                // Follow symlinks (parity with the former grep tool); the `ignore`
                // walker performs its own loop/cycle detection so this is safe.
                wb.follow_links(true);
                if let Some(inc) = include.as_deref() {
                    if let Ok(built) = ov_build(&base, inc) {
                        wb.overrides(built);
                    }
                }
                for entry in wb.build() {
                    if collector.results.len() >= collector.max {
                        break;
                    }
                    let entry = match entry {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                        continue;
                    }
                    collector.rel = rel_path(&base, entry.path());
                    let _ = searcher.search_path(&matcher, entry.path(), &mut collector);
                }
            }

            if collector.results.is_empty() {
                return ToolOutput::ok("no matches");
            }
            let mut out = collector.results.join("\n");
            if collector.results.len() >= collector.max {
                out.push_str(&format!("\n(truncated at {MAX_MATCHES} matches)"));
            }
            truncate_output(out, max_output)
        })
        .await
        .unwrap_or_else(|e| ToolOutput::err(format!("search task failed: {e}")));

        Ok(out)
    }
}

/// Build an `ignore::Override` whitelist from a single include glob. Kept
/// separate from the call site so the borrow checker is happy (the builder
/// is constructed and consumed in one expression).
fn ov_build(base: &Path, inc: &str) -> Result<ignore::overrides::Override, ignore::Error> {
    let mut ov = OverrideBuilder::new(base);
    ov.add(inc)?;
    ov.build()
}

/// Strip the search root so output paths are repo-relative.
fn rel_path(base: &Path, p: &Path) -> String {
    p.strip_prefix(base)
        .map(|x| x.display().to_string())
        .unwrap_or_else(|_| p.display().to_string())
}

/// `Sink` that collects `path:line: content` lines in memory.
struct Collector {
    results: Vec<String>,
    rel: String,
    max: usize,
}

impl Sink for Collector {
    type Error = io::Error;
    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, io::Error> {
        if self.results.len() >= self.max {
            return Ok(false);
        }
        let line = mat.line_number().unwrap_or(0);
        let text = String::from_utf8_lossy(mat.bytes());
        let text = text.trim_end_matches(['\r', '\n']);
        self.results
            .push(format!("{}:{}: {}", self.rel, line, text));
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencoder_core::Tool;
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
            tools_path: None,
        }
    }

    #[tokio::test]
    async fn search_finds_matching_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("greet.rs")).unwrap();
        writeln!(f, "fn main() {{").unwrap();
        writeln!(f, "    println!(\"hello world\");").unwrap();
        writeln!(f, "}}").unwrap();
        let ctx = ctx_for(&dir);
        let tool = SearchTool;
        let out = tool
            .execute(json!({ "pattern": "hello world" }), &ctx)
            .await
            .unwrap();
        assert!(!out.is_error, "expected success, got: {}", out.content);
        // Match line is line 2 in `greet.rs`; expect `greet.rs:2: ...hello world...`.
        assert!(
            out.content.contains("greet.rs:2:"),
            "expected match path:line marker, got: {}",
            out.content
        );
        assert!(
            out.content.contains("hello world"),
            "expected match content, got: {}",
            out.content
        );
    }

    #[tokio::test]
    async fn search_no_matches_returns_ok() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = std::fs::File::create(dir.path().join("f.txt")).unwrap();
        writeln!(f, "alpha").unwrap();
        writeln!(f, "beta").unwrap();
        let ctx = ctx_for(&dir);
        let tool = SearchTool;
        let out = tool
            .execute(json!({ "pattern": "this_pattern_does_not_exist" }), &ctx)
            .await
            .unwrap();
        assert!(!out.is_error, "no matches is not an error");
        assert_eq!(out.content, "no matches");
    }

    #[tokio::test]
    async fn search_empty_pattern_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx_for(&dir);
        let tool = SearchTool;
        let out = tool.execute(json!({ "pattern": "" }), &ctx).await.unwrap();
        assert!(out.is_error, "empty pattern must be an error");
        assert!(out.content.contains("non-empty"));
    }
}
