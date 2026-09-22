//! Ripgrep-library code search.
//!
//! Built on the ripgrep engine crates (`grep-regex` + `grep-searcher`) and the
//! `ignore` walker, so the user needs no `rg` binary installed: the matching is
//! the same in-process engine, and `.gitignore` / `.ignore` / hidden files are
//! honoured exactly as ripgrep does by default.

use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use async_trait::async_trait;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{Searcher, Sink, SinkMatch};
use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use opencoder_core::{json, tool::truncate_output, Tool, ToolContext, ToolOutput};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

mod bounded;

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
        let out = bounded::run(move |cancel| -> ToolOutput {
            let matcher = match RegexMatcherBuilder::new().build(&pattern) {
                Ok(m) => m,
                Err(e) => return ToolOutput::err(format!("invalid regex: {e}")),
            };

            let mut collector = Collector {
                results: Vec::new(),
                rel: String::new(),
                max: MAX_MATCHES,
                bytes: 0,
                truncated: false,
                cancel: cancel.clone(),
            };
            let mut searcher = bounded::searcher();

            if let Err(error) = base.metadata() {
                return ToolOutput::err(format!("search {}: {error}", base.display()));
            }

            if base.is_file() {
                collector.rel = path_str
                    .clone()
                    .unwrap_or_else(|| base.display().to_string());
                if let Err(error) =
                    bounded::search_file(&mut searcher, &matcher, &base, &mut collector, &cancel)
                {
                    return ToolOutput::err(format!("search {}: {error}", base.display()));
                }
            } else {
                let mut wb = WalkBuilder::new(&base);
                // Follow symlinks (parity with the former grep tool), but never
                // re-enter a directory: the walker's built-in loop detection only
                // catches a directory reappearing in its own ancestor chain. Links
                // whose hops are distinct directories (`/proc/<pid>/root` resolves
                // to `/`, sysfs `subsystem/devices` chains grow new paths every
                // hop) defeat it and expand the walk exponentially without ever
                // terminating. The `dir_first_visit` guard prunes any physical
                // directory already visited once, bounding the walk to one visit
                // per directory while still following links.
                wb.follow_links(true);
                let visited: Arc<Mutex<HashSet<(u64, u64)>>> = Arc::new(Mutex::new(HashSet::new()));
                let walking = cancel.clone();
                wb.filter_entry(move |e| !walking.is_cancelled() && dir_first_visit(e, &visited));
                if let Some(inc) = include.as_deref() {
                    if let Ok(built) = ov_build(&base, inc) {
                        wb.overrides(built);
                    }
                }
                for entry in wb.build() {
                    if cancel.is_cancelled() || collector.full() {
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
                    if let Err(error) = bounded::search_file(
                        &mut searcher,
                        &matcher,
                        entry.path(),
                        &mut collector,
                        &cancel,
                    ) {
                        return ToolOutput::err(format!(
                            "search {}: {error}",
                            entry.path().display()
                        ));
                    }
                }
            }

            if collector.results.is_empty() {
                return ToolOutput::ok("no matches");
            }
            let mut out = collector.results.join("\n");
            if collector.results.len() >= collector.max {
                out.push_str(&format!("\n(truncated at {MAX_MATCHES} matches)"));
            }
            if collector.truncated {
                out.push_str("\n(truncated at search output byte limit)");
            }
            truncate_output(out, max_output)
        })
        .await;

        Ok(out)
    }
}

/// Re-entry guard for symlink-following walks: returns `true` only the first
/// time a physical directory (identified by `(dev, ino)`) is seen. Symlinks
/// may be followed, but a directory already visited anywhere earlier in the
/// walk is pruned, so no directory is ever re-entered. Without this, link
/// chains such as `/proc/<pid>/root` (distinct hop dirs, same target `/`)
/// turn the walk into an unbounded, core-pinning traversal.
fn dir_first_visit(e: &ignore::DirEntry, visited: &Mutex<HashSet<(u64, u64)>>) -> bool {
    if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match e.metadata() {
            Ok(md) => {
                let mut seen = visited.lock().expect("search walk dedup lock poisoned");
                seen.insert((md.dev(), md.ino()))
            }
            // Un-stat-able directory: let the walker surface the error itself.
            Err(_) => true,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = visited;
        true
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
    bytes: usize,
    truncated: bool,
    cancel: CancellationToken,
}

impl Collector {
    fn full(&self) -> bool {
        self.results.len() >= self.max || self.truncated
    }
}

impl Sink for Collector {
    type Error = io::Error;
    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, io::Error> {
        if self.cancel.is_cancelled() || self.full() {
            return Ok(false);
        }
        let line = mat.line_number().unwrap_or(0);
        let text = String::from_utf8_lossy(mat.bytes());
        let text = text.trim_end_matches(['\r', '\n']);
        let result = format!("{}:{}: {}", self.rel, line, text);
        let remaining = bounded::OUTPUT_BYTES.saturating_sub(self.bytes);
        let mut end = result.len().min(remaining);
        while !result.is_char_boundary(end) {
            end -= 1;
        }
        self.truncated = end < result.len() || result.len() >= remaining;
        self.bytes += end;
        self.results.push(result[..end].to_owned());
        Ok(!self.full())
    }
}

#[cfg(test)]
mod limits_tests;
#[cfg(test)]
mod tests;
