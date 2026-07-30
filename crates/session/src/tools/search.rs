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
        let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
        if pattern.is_empty() {
            return Ok(ToolOutput::err("search requires a non-empty 'pattern'"));
        }
        let matcher = match RegexMatcherBuilder::new().build(pattern) {
            Ok(m) => m,
            Err(e) => return Ok(ToolOutput::err(format!("invalid regex: {e}"))),
        };

        let path_str = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let include = input.get("include").and_then(|v| v.as_str());
        let base = match &path_str {
            Some(p) => super::read::resolve(ctx, p),
            None => ctx.working_dir.clone(),
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
            if let Some(inc) = include {
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
            return Ok(ToolOutput::ok("no matches"));
        }
        let mut out = collector.results.join("\n");
        if collector.results.len() >= collector.max {
            out.push_str(&format!("\n(truncated at {MAX_MATCHES} matches)"));
        }
        Ok(truncate_output(out, ctx.max_output))
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
