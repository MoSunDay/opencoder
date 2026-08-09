//! File-tree model for the notepad explorer panel.
//!
//! Recursively scans the workdir (skipping noise dirs like `.git`, `target`,
//! `node_modules`), flattens to a visible list honouring collapse state, and
//! renders with indentation + directory glyphs.

use std::path::{Path, PathBuf};

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::theme;

/// Dirs that are never shown in the tree.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".next",
    "dist",
    "build",
    "__pycache__",
    ".cache",
    ".opencode",
];

/// Maximum recursion depth — safety net against pathological deep trees.
const MAX_DEPTH: usize = 32;
/// Maximum total entries in the flat list — safety net against huge dirs.
const MAX_TOTAL_ENTRIES: usize = 5000;

/// A single visible row in the flattened tree.
#[derive(Clone, Debug)]
pub struct FlatNode {
    pub name: String,
    pub path: PathBuf,
    pub depth: usize,
    pub is_dir: bool,
    pub collapsed: bool,
}

/// Mutable tree state: visible rows, selection index, scroll offset, and an
/// optional inline mini-input (create-file / delete-confirm).
#[derive(Clone, Debug)]
pub struct TreeState {
    pub flat: Vec<FlatNode>,
    pub selected: usize,
    pub scroll: usize,
    pub input: Option<TreeInput>,
    workdir: PathBuf,
}

/// Inline input modes for the tree panel.
#[derive(Clone, Debug)]
pub enum TreeInput {
    /// Creating a new file — `buf` accumulates the name, `parent` is the
    /// directory it will be created in.
    Create { buf: String, parent: PathBuf },
    /// Delete confirmation — `path` is the file/dir to remove.
    DeleteConfirm { path: PathBuf },
}

impl TreeState {
    pub fn new(workdir: &Path) -> Self {
        let mut s = Self {
            flat: Vec::new(),
            selected: 0,
            scroll: 0,
            input: None,
            workdir: workdir.to_path_buf(),
        };
        s.rebuild(workdir);
        s
    }

    /// Rescan the workdir and rebuild the flat list, preserving collapse state
    /// for paths that still exist.
    pub fn rebuild(&mut self, workdir: &Path) {
        // Collect expanded dirs (lazy: dirs start collapsed by default).
        let mut expanded: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
        for n in &self.flat {
            if n.is_dir && !n.collapsed {
                expanded.insert(n.path.clone());
            }
        }
        let mut flat = Vec::new();
        build_recursive(workdir, 0, &expanded, &mut flat);
        self.flat = flat;
        if self.selected >= self.flat.len() {
            self.selected = self.flat.len().saturating_sub(1);
        }
        self.scroll = 0;
    }

    pub fn move_cursor(&mut self, delta: i32) {
        if self.flat.is_empty() {
            return;
        }
        let len = self.flat.len();
        let new = (self.selected as i32 + delta).clamp(0, len as i32 - 1) as usize;
        self.selected = new;
    }

    /// Toggle collapse on the selected directory (no-op for files).
    pub fn toggle_collapse(&mut self) {
        if let Some(n) = self.flat.get_mut(self.selected) {
            if n.is_dir {
                n.collapsed = !n.collapsed;
            }
        }
        let wd = self.workdir.clone();
        self.rebuild(&wd);
    }

    pub fn collapse_dir(&mut self) {
        if let Some(n) = self.flat.get_mut(self.selected) {
            if n.is_dir {
                n.collapsed = true;
            }
        }
        let wd = self.workdir.clone();
        self.rebuild(&wd);
    }

    pub fn selected_node(&self) -> Option<&FlatNode> {
        self.flat.get(self.selected)
    }

    /// Scroll so the selection is visible within `visible_h` rows.
    pub fn ensure_visible(&mut self, visible_h: usize) {
        if self.flat.is_empty() || visible_h == 0 {
            return;
        }
        let h = visible_h.saturating_sub(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected > self.scroll + h {
            self.scroll = self.selected.saturating_sub(h);
        }
    }
}

fn build_recursive(
    dir: &Path,
    depth: usize,
    expanded: &std::collections::HashSet<PathBuf>,
    out: &mut Vec<FlatNode>,
) {
    if depth >= MAX_DEPTH {
        return;
    }
    let mut entries: Vec<(String, PathBuf, bool)> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let ft = e.file_type().ok()?;
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') && depth == 0 {
                    // show dotfiles at top level but skip hidden noise dirs
                    if SKIP_DIRS.contains(&name.as_str()) {
                        return None;
                    }
                }
                if ft.is_dir() && SKIP_DIRS.contains(&name.as_str()) {
                    return None;
                }
                Some((name, e.path(), ft.is_dir()))
            })
            .collect(),
        Err(_) => return,
    };
    // dirs first, then files; alphabetical within each group.
    entries.sort_by(|a, b| match (a.2, b.2) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.0.to_lowercase().cmp(&b.0.to_lowercase()),
    });

    for (name, path, is_dir) in entries {
        if out.len() >= MAX_TOTAL_ENTRIES {
            out.push(FlatNode {
                name: "\u{2026} (truncated)".to_string(),
                path: dir.to_path_buf(),
                depth,
                is_dir: false,
                collapsed: false,
            });
            return;
        }
        let is_collapsed = !expanded.contains(&path);
        out.push(FlatNode {
            name: name.clone(),
            path: path.clone(),
            depth,
            is_dir,
            collapsed: is_collapsed,
        });
        if is_dir && !is_collapsed {
            build_recursive(&path, depth + 1, expanded, out);
        }
    }
}

/// Render the file tree panel.
pub fn render_tree(f: &mut Frame, area: Rect, state: &TreeState, focused: bool) {
    let title = " Explorer ";
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
    if state.flat.len() > vis_h && state.selected >= scroll + vis_h {
        scroll = state.selected.saturating_sub(vis_h - 1);
    }
    if state.selected < scroll {
        scroll = state.selected;
    }
    let end = (scroll + vis_h).min(state.flat.len());
    let start = scroll.min(end);

    let mut lines: Vec<Line> = Vec::new();
    for (i, node) in state.flat[start..end].iter().enumerate() {
        let abs_idx = start + i;
        let indent = "  ".repeat(node.depth);
        let glyph = if node.is_dir {
            if node.collapsed {
                "▸ "
            } else {
                "▾ "
            }
        } else {
            "  "
        };
        let style = if abs_idx == state.selected {
            Style::default().bg(theme::highlight_bg())
        } else {
            Style::default()
        };
        let name_style = if node.is_dir {
            Style::default().fg(theme::accent())
        } else {
            Style::default().fg(theme::text())
        };
        let mut spans = vec![Span::raw(format!("{}{}", indent, glyph))];
        spans.push(Span::styled(node.name.clone(), name_style));
        lines.push(Line::from(spans).style(style));
    }

    // Inline input prompt at the bottom.
    if let Some(inp) = &state.input {
        lines.push(Line::raw(""));
        match inp {
            TreeInput::Create { buf, .. } => {
                lines.push(Line::styled(
                    format!(" new file: {}_", buf),
                    Style::default().fg(theme::ok_color()),
                ));
            }
            TreeInput::DeleteConfirm { path } => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                lines.push(Line::styled(
                    format!(" delete '{}'? y/n", name),
                    Style::default().fg(theme::err_color()),
                ));
            }
        }
    }

    if lines.is_empty() {
        lines.push(Line::styled(
            " (empty)".to_string(),
            Style::default().fg(theme::subtle()),
        ));
    }
    let para = Paragraph::new(lines).scroll((0, 0));
    f.render_widget(para, inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn mk_tmp() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("src/sub")).unwrap();
        fs::write(d.path().join("src/main.rs"), "fn main(){}").unwrap();
        fs::write(d.path().join("src/lib.rs"), "").unwrap();
        fs::write(d.path().join("README.md"), "# hi").unwrap();
        fs::create_dir_all(d.path().join("tests")).unwrap();
        fs::write(d.path().join("tests/integration.rs"), "test").unwrap();
        d
    }

    #[test]
    fn basic_structure() {
        let d = mk_tmp();
        let st = TreeState::new(d.path());
        // Lazy: dirs start collapsed, children not yet loaded.
        let names: Vec<&str> = st.flat.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"src"));
        assert!(names.contains(&"README.md"));
        // noise dirs filtered
        assert!(!names.contains(&".git"));
        assert!(!names.contains(&"target"));
        // children not visible until expanded
        assert!(!names.contains(&"main.rs"));
        assert!(!names.contains(&"lib.rs"));
        // depth of top-level entries is 0
        let src = st.flat.iter().find(|n| n.name == "src").unwrap();
        assert_eq!(src.depth, 0);
        assert!(src.is_dir);
        assert!(src.collapsed);
    }

    #[test]
    fn dirs_before_files() {
        let d = mk_tmp();
        let st = TreeState::new(d.path());
        let first = &st.flat[0];
        assert!(first.is_dir);
        assert_eq!(first.name, "src");
    }

    #[test]
    fn expand_shows_children() {
        let d = mk_tmp();
        let mut st = TreeState::new(d.path());
        // src starts collapsed at index 0
        assert!(st.flat[0].collapsed);
        assert!(!st.flat.iter().any(|n| n.name == "main.rs"));
        // expand src
        st.selected = 0;
        st.toggle_collapse();
        assert!(!st.flat[0].collapsed);
        let names: Vec<&str> = st.flat.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"main.rs"));
        assert!(names.contains(&"lib.rs"));
        let main = st.flat.iter().find(|n| n.name == "main.rs").unwrap();
        assert_eq!(main.depth, 1);
    }

    #[test]
    fn collapse_hides_children() {
        let d = mk_tmp();
        let mut st = TreeState::new(d.path());
        // expand src first
        st.selected = 0;
        st.toggle_collapse();
        assert!(!st.flat[0].collapsed);
        // now collapse
        st.toggle_collapse();
        assert!(st.flat[0].collapsed);
        let names: Vec<&str> = st.flat.iter().map(|n| n.name.as_str()).collect();
        assert!(!names.contains(&"main.rs"));
    }

    #[test]
    fn move_cursor_bounds() {
        let d = mk_tmp();
        let mut st = TreeState::new(d.path());
        let len = st.flat.len();
        st.move_cursor(100);
        assert_eq!(st.selected, len - 1);
        st.move_cursor(-100);
        assert_eq!(st.selected, 0);
    }

    #[test]
    fn rebuild_preserves_expansion() {
        let d = mk_tmp();
        let mut st = TreeState::new(d.path());
        // src starts collapsed; expand it
        st.selected = 0;
        st.toggle_collapse();
        assert!(!st.flat[0].collapsed);
        st.rebuild(d.path());
        assert!(!st.flat[0].collapsed);
    }

    #[test]
    fn ensure_visible_adjusts_scroll() {
        let d = mk_tmp();
        let mut st = TreeState::new(d.path());
        st.selected = 5;
        st.scroll = 0;
        st.ensure_visible(3);
        assert!(st.scroll > 0);
    }

    #[test]
    fn depth_limit_truncates() {
        let d = tempfile::tempdir().unwrap();
        // Create a tree deeper than MAX_DEPTH (32).
        let mut p = d.path().to_path_buf();
        for i in 0..40 {
            p = p.join(format!("d{}", i));
            fs::create_dir_all(&p).unwrap();
        }
        let mut st = TreeState::new(d.path());
        // Expand all levels until we hit the limit — no panic.
        for _ in 0..40 {
            if let Some(n) = st.selected_node() {
                if n.is_dir && n.collapsed {
                    st.toggle_collapse();
                    continue;
                }
            }
            break;
        }
        // Should not have crashed.
        assert!(!st.flat.is_empty());
    }

    #[test]
    fn entry_limit_truncates() {
        let d = tempfile::tempdir().unwrap();
        // Create more files than MAX_TOTAL_ENTRIES.
        for i in 0..6000 {
            fs::write(d.path().join(format!("file_{:04}.txt", i)), "").unwrap();
        }
        let st = TreeState::new(d.path());
        assert!(st.flat.len() <= MAX_TOTAL_ENTRIES + 1);
        assert!(st.flat.iter().any(|n| n.name.contains("truncated")));
    }
}
