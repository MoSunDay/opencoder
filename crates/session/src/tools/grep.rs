use anyhow::Result;
use async_trait::async_trait;
use opencoder_core::{json, Tool, ToolContext, ToolOutput};
use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Searches file contents with a regex. Returns matching lines with file:line prefixes. Searches recursively under the given path (default working dir)."
    }
    fn parameters(&self) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "pattern".into(),
            json::prop_str("Regular expression to search for."),
        );
        props.insert(
            "path".into(),
            json::prop_str("Optional directory or file to search in."),
        );
        props.insert(
            "include".into(),
            json::prop_str("Optional glob filter for file names, e.g. \"*.rs\"."),
        );
        json::object_schema(Value::Object(props), &["pattern"])
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
        let re = match Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return Ok(ToolOutput::err(format!("invalid regex: {e}"))),
        };
        let base = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| ctx.working_dir.display().to_string());
        let include = input.get("include").and_then(|v| v.as_str());
        let inc_re = include.map(glob_to_regex);
        let root = Path::new(&base);
        let mut results: Vec<String> = Vec::new();
        let mut visited = 0u32;
        let mut seen = HashSet::new();
        walk(root, &re, &inc_re, &mut results, &mut visited, 1000, &mut seen);
        if results.is_empty() {
            return Ok(ToolOutput::ok("no matches"));
        }
        let out = results.join("\n");
        Ok(opencoder_core::tool::truncate_output(out, ctx.max_output))
    }
}

fn walk(
    dir: &Path,
    re: &Regex,
    inc_re: &Option<Regex>,
    out: &mut Vec<String>,
    visited: &mut u32,
    cap: usize,
    seen: &mut HashSet<PathBuf>,
) {
    if *visited > 50_000 || out.len() >= cap {
        return;
    }
    // Break symlink cycles: record the real (canonical) directory path. A
    // symlink pointing at an ancestor resolves to an already-seen canonical
    // path and is skipped, instead of recursing until the file-count cap.
    let canon = match dir.canonicalize() {
        Ok(c) => c,
        Err(_) => return,
    };
    if !seen.insert(canon) {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if matches!(
                name.as_ref(),
                ".git" | "node_modules" | "target" | "dist" | ".next" | ".cache"
            ) {
                continue;
            }
            walk(&path, re, inc_re, out, visited, cap, seen);
        } else if path.is_file() {
            *visited += 1;
            if let Some(inc) = inc_re {
                if !inc.is_match(&name) {
                    continue;
                }
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                for (i, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        if out.len() >= cap {
                            return;
                        }
                        out.push(format!("{}:{}: {}", path.display(), i + 1, line.trim_end()));
                    }
                }
            }
        }
    }
}

fn glob_to_regex(glob: &str) -> Regex {
    let mut s = String::from("^");
    for ch in glob.chars() {
        match ch {
            '*' => s.push_str(".*"),
            '?' => s.push('.'),
            c if "\\.+()[]{}|^$".contains(c) => {
                s.push('\\');
                s.push(c);
            }
            c => s.push(c),
        }
    }
    s.push('$');
    Regex::new(&s).unwrap_or_else(|_| Regex::new("^.*$").unwrap())
}
