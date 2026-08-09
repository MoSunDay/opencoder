//! File-content search for the notepad explorer.
//!
//! Uses `ripgrep` (`rg`) when available, falling back to `grep -rn`. Results
//! are collected into a flat list that the user can navigate and open in the
//! editor panel.

use std::path::{Path, PathBuf};

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::theme;

const MAX_RESULTS: usize = 200;

/// One search hit.
#[derive(Clone, Debug)]
pub struct SearchHit {
    pub path: PathBuf,
    pub line_no: usize,
    pub text: String,
}

/// Search panel state.
#[derive(Clone, Debug)]
pub struct SearchState {
    pub query: String,
    pub results: Vec<SearchHit>,
    pub selected: usize,
    pub scroll: usize,
    /// While `true`, typed characters edit `query` instead of navigating
    /// results.
    pub editing: bool,
    /// Transient status / error message.
    pub status: String,
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            scroll: 0,
            editing: true,
            status: String::new(),
        }
    }

    pub fn move_cursor(&mut self, delta: i32) {
        if self.results.is_empty() {
            return;
        }
        let len = self.results.len();
        let new = (self.selected as i32 + delta).clamp(0, len as i32 - 1) as usize;
        self.selected = new;
    }

    pub fn selected_hit(&self) -> Option<&SearchHit> {
        self.results.get(self.selected)
    }

    /// Ensure the selection is visible in `vis_h` rows.
    pub fn ensure_visible(&mut self, vis_h: usize) {
        if self.results.is_empty() || vis_h == 0 {
            return;
        }
        let h = vis_h.saturating_sub(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected > self.scroll + h {
            self.scroll = self.selected.saturating_sub(h);
        }
    }
}

impl Default for SearchState {
    fn default() -> Self {
        Self::new()
    }
}

/// Run a content search in `workdir`. Tries `rg`, falls back to `grep -rn`.
pub async fn search(query: &str, workdir: &Path) -> Result<Vec<SearchHit>, String> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    // Try ripgrep first.
    match try_rg(query, workdir).await {
        Ok(hits) => Ok(hits),
        Err(_) => {
            // Fallback to grep.
            try_grep(query, workdir)
                .await
                .map_err(|e| format!("rg and grep both failed: {}", e))
        }
    }
}

async fn try_rg(query: &str, workdir: &Path) -> Result<Vec<SearchHit>, String> {
    let out = tokio::process::Command::new("rg")
        .arg("--line-number")
        .arg("--no-heading")
        .arg("--color=never")
        .arg("--max-count=500")
        .arg(query)
        .current_dir(workdir)
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if !out.status.success() && out.stdout.is_empty() {
        // rg returns exit code 1 for "no matches" — not an error.
        if out.stderr.is_empty() {
            return Ok(Vec::new());
        }
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    Ok(parse_hits(&out.stdout))
}

async fn try_grep(query: &str, workdir: &Path) -> Result<Vec<SearchHit>, String> {
    let out = tokio::process::Command::new("grep")
        .arg("-rn")
        .arg("--color=never")
        .arg(query)
        .arg(".")
        .current_dir(workdir)
        .output()
        .await
        .map_err(|e| e.to_string())?;

    Ok(parse_hits_grep(&out.stdout, workdir))
}

/// Parse `rg` output: `path:line:content`.
fn parse_hits(raw: &[u8]) -> Vec<SearchHit> {
    let text = String::from_utf8_lossy(raw);
    let mut hits = Vec::new();
    for line in text.lines() {
        if let Some(hit) = parse_rg_line(line) {
            hits.push(hit);
            if hits.len() >= MAX_RESULTS {
                break;
            }
        }
    }
    hits
}

fn parse_rg_line(line: &str) -> Option<SearchHit> {
    let mut parts = line.splitn(3, ':');
    let path = parts.next()?.to_string();
    let line_no: usize = parts.next()?.parse().ok()?;
    let text = parts.next()?.to_string();
    Some(SearchHit {
        path: PathBuf::from(path),
        line_no,
        text,
    })
}

/// Parse `grep -rn .` output: `./path:line:content`.
fn parse_hits_grep(raw: &[u8], workdir: &Path) -> Vec<SearchHit> {
    let text = String::from_utf8_lossy(raw);
    let mut hits = Vec::new();
    for line in text.lines() {
        if let Some(mut hit) = parse_rg_line(line) {
            // Strip leading "./" and resolve relative to workdir.
            let p = hit.path.strip_prefix(".").unwrap_or(&hit.path).to_path_buf();
            hit.path = if p.is_absolute() {
                p
            } else {
                workdir.join(p)
            };
            hits.push(hit);
            if hits.len() >= MAX_RESULTS {
                break;
            }
        }
    }
    hits
}

/// Render the search overlay panel.
pub fn render_search(f: &mut Frame, area: Rect, state: &SearchState, focused: bool) {
    let title = if state.editing {
        " Search (rg/grep) "
    } else {
        " Search Results "
    };
    let block = if focused {
        theme::rounded_block_focus(title)
    } else {
        theme::rounded_block(title)
    };
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let vis_h = inner.height as usize;
    let mut scroll = state.scroll;
    if state.results.len() > vis_h.saturating_sub(2) && state.selected >= scroll + vis_h {
        scroll = state.selected.saturating_sub(vis_h.saturating_sub(3));
    }
    if state.selected < scroll {
        scroll = state.selected;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Query line at the top.
    let q = if state.editing {
        format!("/{}_ ", state.query)
    } else {
        format!("/{}  ({} hits)", state.query, state.results.len())
    };
    lines.push(Line::from(Span::styled(
        q,
        Style::default().fg(theme::accent()),
    )));
    lines.push(Line::raw(""));

    if !state.status.is_empty() {
        lines.push(Line::from(Span::styled(
            state.status.clone(),
            Style::default().fg(theme::err_color()),
        )));
    }

    if state.results.is_empty() && state.status.is_empty() {
        lines.push(Line::from(Span::styled(
            " (no results)".to_string(),
            Style::default().fg(theme::subtle()),
        )));
    }

    let avail = vis_h.saturating_sub(lines.len());
    let start = scroll.min(state.results.len());
    let end = (start + avail).min(state.results.len());

    for (i, hit) in state.results[start..end].iter().enumerate() {
        let abs = start + i;
        let fname = hit
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let loc = format!("{}:{}", fname, hit.line_no);
        let style = if abs == state.selected {
            Style::default().bg(theme::highlight_bg())
        } else {
            Style::default()
        };
        let mut spans = vec![Span::styled(
            format!(" {} ", loc),
            Style::default().fg(theme::info_color()),
        )];
        // Truncate hit text to fit.
        let max = 60usize;
        let display = if hit.text.len() > max {
            format!("{}...", &hit.text.trim()[..max.min(hit.text.trim().len())])
        } else {
            hit.text.trim().to_string()
        };
        spans.push(Span::raw(display));
        lines.push(Line::from(spans).style(style));
    }

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn search_finds_content() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.txt"), "hello world\nfoo bar").unwrap();
        let hits = search("hello", d.path()).await.unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].line_no, 1);
        assert!(hits[0].text.contains("hello"));
    }

    #[tokio::test]
    async fn search_empty_query_returns_empty() {
        let d = tempfile::tempdir().unwrap();
        let hits = search("", d.path()).await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn search_no_match() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.txt"), "hello").unwrap();
        let hits = search("nonexistent_xyz123", d.path()).await.unwrap();
        assert!(hits.is_empty());
    }

    #[tokio::test]
    async fn search_multiline_file() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join("multi.rs"),
            "fn foo() {}\nfn bar() {}\nfn baz() {}\n",
        )
        .unwrap();
        let hits = search("fn", d.path()).await.unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].line_no, 1);
        assert_eq!(hits[2].line_no, 3);
    }

    #[test]
    fn parse_rg_line_basic() {
        let h = parse_rg_line("src/main.rs:42:fn main() {").unwrap();
        assert_eq!(h.path, PathBuf::from("src/main.rs"));
        assert_eq!(h.line_no, 42);
        assert_eq!(h.text, "fn main() {");
    }

    #[test]
    fn parse_rg_line_with_colons_in_text() {
        let h = parse_rg_line("a.txt:1:foo:bar:baz").unwrap();
        assert_eq!(h.line_no, 1);
        assert_eq!(h.text, "foo:bar:baz");
    }

    #[test]
    fn parse_rg_line_invalid() {
        assert!(parse_rg_line("nocolons").is_none());
        assert!(parse_rg_line("only:two").is_none());
    }

    #[test]
    fn search_state_cursor_bounds() {
        let mut s = SearchState::new();
        s.editing = false;
        s.results = vec![
            SearchHit { path: PathBuf::from("a"), line_no: 1, text: "t".into() },
            SearchHit { path: PathBuf::from("b"), line_no: 2, text: "t".into() },
        ];
        s.move_cursor(100);
        assert_eq!(s.selected, 1);
        s.move_cursor(-100);
        assert_eq!(s.selected, 0);
    }
}
