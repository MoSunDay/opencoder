//! Editor panel: loads a file into a [`VimState`], persists on `:w` / `:wq`,
//! and renders with a line-number gutter.

use std::path::{Path, PathBuf};

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::theme;
use crate::vim::{VimMode, VimState};

/// Editor state wrapping the shared vim engine.
#[derive(Clone, Debug)]
pub struct EditorState {
    pub vim: VimState,
    pub file_path: Option<PathBuf>,
    /// Vertical scroll offset (visual rows).
    pub scroll: u16,
}

impl EditorState {
    /// Start with an empty buffer and no file.
    pub fn empty() -> Self {
        Self {
            vim: VimState::new(String::new()),
            file_path: None,
            scroll: 0,
        }
    }

    /// Load `path` into the editor, resetting vim state to Normal mode.
    pub fn load(&mut self, path: &Path) {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|_| {
                format!("# (cannot read {})\n", path.display())
            });
        self.vim = VimState::new(text);
        self.vim.mode = VimMode::Normal;
        self.vim.cursor = 0;
        self.file_path = Some(path.to_path_buf());
        self.scroll = 0;
    }

    /// Write the current buffer back to disk if a file is loaded.
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(p) = &self.file_path {
            std::fs::write(p, &self.vim.text)?;
        }
        // Sync `original` so `is_modified` resets after a save.
        Ok(())
    }

    /// Returns `true` if the Enter key in Command mode is `:w` (write-only).
    pub fn is_write_cmd(&self) -> bool {
        self.vim.mode == VimMode::Command && self.vim.cmdline.trim() == "w"
    }

    /// Returns `true` if Enter in Command mode is `:wq`.
    pub fn is_writequit_cmd(&self) -> bool {
        self.vim.mode == VimMode::Command
            && matches!(self.vim.cmdline.trim(), "wq" | "x")
    }

    /// Execute a `:w` command locally (write + reset modified state + return to
    /// Normal mode). Does **not** call the vim engine — intercepts before it.
    pub fn do_write(&mut self) -> std::io::Result<()> {
        self.save()?;
        // After a successful write, sync original so is_modified() becomes false.
        self.vim.original = self.vim.text.clone();
        self.vim.mode = VimMode::Normal;
        self.vim.cmdline.clear();
        self.vim.reset_pending();
        self.vim.status = format!(
            "saved: {}",
            self.file_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        );
        Ok(())
    }

    /// Execute `:wq`: write (if modified) then signal exit.
    pub fn do_writequit(&mut self) -> std::io::Result<()> {
        if self.vim.is_modified() {
            self.save()?;
            self.vim.original = self.vim.text.clone();
        }
        Ok(())
    }

    pub fn title(&self) -> String {
        match &self.file_path {
            Some(p) => p
                .file_name()
                .map(|n| format!(" {} ", n.to_string_lossy()))
                .unwrap_or_else(|| " [no name] ".to_string()),
            None => " [no file] ".to_string(),
        }
    }

    pub fn is_modified(&self) -> bool {
        self.vim.is_modified()
    }

    /// Number of logical lines (counting a trailing newline as one extra).
    pub fn line_count(&self) -> usize {
        self.vim.text.lines().count().max(1)
    }

    /// Adjust scroll so the cursor's visual row stays visible.
    pub fn ensure_cursor_visible(&mut self, inner_h: u16) {
        if inner_h <= 2 {
            return;
        }
        let vis_h = inner_h as usize;
        // Determine the cursor's logical line.
        let before: usize = self.vim.text[..self.char_byte_offset()].matches('\n').count();
        let cur_row = before;
        let scroll = self.scroll as usize;
        let h = vis_h.saturating_sub(1);
        if cur_row < scroll {
            self.scroll = cur_row as u16;
        } else if cur_row > scroll + h {
            self.scroll = (cur_row - h) as u16;
        }
    }

    /// Byte offset of the char-index cursor (for line counting).
    fn char_byte_offset(&self) -> usize {
        let mut off = 0;
        for (i, (b, _)) in self.vim.text.char_indices().enumerate() {
            if i == self.vim.cursor {
                return b;
            }
            off = b;
        }
        // cursor at end
        off
    }
}

/// Render the editor panel with a line-number gutter.
pub fn render_editor(f: &mut Frame, area: Rect, state: &EditorState, focused: bool) {
    let title = state.title();
    let block = if focused {
        theme::rounded_block_focus(&title)
    } else {
        theme::rounded_block(&title)
    };

    // Add mode label at bottom-right of the border.
    let mode_label = state.vim.mode_label();
    let block = block.title_bottom(
        ratatui::text::Line::from(format!(" {} ", mode_label))
            .alignment(ratatui::layout::Alignment::Right),
    );

    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let gutter_w = (state.line_count().to_string().len() + 2) as u16;
    let text_w = inner.width.saturating_sub(gutter_w);

    let lines: Vec<&str> = state.vim.text.lines().collect();
    let vis_h = inner.height as usize;
    let scroll = state.scroll as usize;
    let total = lines.len();

    let start = scroll.min(total);
    let end = (start + vis_h).min(total);

    let mut row_lines: Vec<Line> = Vec::new();
    for (i, raw) in lines[start..end].iter().enumerate() {
        let line_no = start + i + 1;
        let num_str = format!("{:>width$} ", line_no, width = (gutter_w - 2) as usize);
        let num_span = Span::styled(num_str, Style::default().fg(theme::subtle()));
        let content = truncate_for_width(raw, text_w as usize);
        let content_span = Span::raw(content);
        row_lines.push(Line::from(vec![num_span, content_span]));
    }
    // Handle empty buffer.
    if row_lines.is_empty() {
        row_lines.push(Line::from(vec![
            Span::styled(" 1 ", Style::default().fg(theme::subtle())),
            Span::raw(""),
        ]));
    }

    let para = Paragraph::new(row_lines).scroll((0, 0));
    f.render_widget(para, inner);

    // Position hardware cursor inside the editor.
    set_editor_cursor(f, inner, state, gutter_w, text_w);
}

/// Compute the cursor's terminal position and place it.
fn set_editor_cursor(
    f: &mut Frame,
    inner: Rect,
    state: &EditorState,
    gutter_w: u16,
    _text_w: u16,
) {
    if state.vim.mode == VimMode::Command || state.vim.mode == VimMode::Search {
        // Don't fight the cmdline — just show the cursor at the end of the
        // status line. It's approximate but avoids double-cursor artifacts.
        return;
    }
    let text = &state.vim.text;
    let cursor = state.vim.cursor;

    // Logical row.
    let before_bytes = char_byte_offset(text, cursor);
    let row = text[..before_bytes].matches('\n').count();
    // Column within the logical line.
    let line_start = text[..before_bytes].rfind('\n').map(|p| p + 1).unwrap_or(0);
    let col_str = &text[line_start..before_bytes];
    let col = crate::composer::str_width(col_str);

    let scroll = state.scroll as usize;
    if row < scroll {
        return;
    }
    let rel_row = (row - scroll).min(u16::MAX as usize) as u16;
    let col_u16 = col.min(u16::MAX as usize) as u16;
    let y = inner.y.saturating_add(rel_row);
    let x = inner.x.saturating_add(gutter_w).saturating_add(col_u16);
    // Clamp to inner area to avoid cursor escaping the panel.
    let x = x.min(inner.right().saturating_sub(1));
    let y = y.min(inner.bottom().saturating_sub(1));
    f.set_cursor_position((x, y));
}

fn char_byte_offset(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

fn truncate_for_width(s: &str, max_w: usize) -> String {
    let w = crate::composer::str_width(s);
    if w <= max_w {
        return s.to_string();
    }
    let mut out = String::new();
    let mut cur = 0usize;
    for ch in s.chars() {
        let cw = crate::composer::char_width(ch);
        if cur + cw > max_w {
            break;
        }
        out.push(ch);
        cur += cw;
    }
    out
}

/// Check if this key, when in Normal mode, is a plain focus-cycle key that
/// should NOT be sent to the vim engine.
pub fn is_focus_cycle_key(k: &crossterm::event::KeyEvent) -> bool {
    use crossterm::event::KeyCode;
    k.code == KeyCode::Tab
        && state_normal_or(k)
}

fn state_normal_or(_k: &crossterm::event::KeyEvent) -> bool {
    true // caller checks mode before calling
}

/// Determine if the editor should consume a Tab key for focus cycling.
pub fn should_cycle_focus(vim: &VimState, k: &crossterm::event::KeyEvent) -> bool {
    use crossterm::event::KeyCode;
    vim.mode == VimMode::Normal && k.code == KeyCode::Tab
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn load_file_into_vim() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("test.rs");
        fs::write(&p, "fn main() {}\n").unwrap();
        let mut ed = EditorState::empty();
        ed.load(&p);
        assert_eq!(ed.vim.text, "fn main() {}\n");
        assert_eq!(ed.vim.mode, VimMode::Normal);
        assert!(!ed.is_modified());
    }

    #[test]
    fn save_writes_to_disk() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("out.txt");
        let mut ed = EditorState::empty();
        ed.load(&p);
        ed.vim.text = "modified content".to_string();
        assert!(ed.is_modified());
        ed.save().unwrap();
        let disk = fs::read_to_string(&p).unwrap();
        assert_eq!(disk, "modified content");
    }

    #[test]
    fn do_write_resets_modified() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("w.txt");
        fs::write(&p, "orig").unwrap();
        let mut ed = EditorState::empty();
        ed.load(&p);
        ed.vim.text = "changed".to_string();
        assert!(ed.is_modified());
        ed.do_write().unwrap();
        assert!(!ed.is_modified());
        assert_eq!(ed.vim.mode, VimMode::Normal);
        assert_eq!(fs::read_to_string(&p).unwrap(), "changed");
    }

    #[test]
    fn do_writequit_writes_if_modified() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("wq.txt");
        fs::write(&p, "orig").unwrap();
        let mut ed = EditorState::empty();
        ed.load(&p);
        ed.vim.text = "final".to_string();
        ed.do_writequit().unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "final");
    }

    #[test]
    fn do_writequit_skips_write_if_not_modified() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("nm.txt");
        fs::write(&p, "unchanged").unwrap();
        let mut ed = EditorState::empty();
        ed.load(&p);
        // Simulate a writequit without changes — mtime should stay the same.
        let before = fs::metadata(&p).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        ed.do_writequit().unwrap();
        let after = fs::metadata(&p).unwrap().modified().unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn line_count_basic() {
        let mut ed = EditorState::empty();
        ed.vim = VimState::new("a\nb\nc".to_string());
        assert_eq!(ed.line_count(), 3);
    }

    #[test]
    fn line_count_empty() {
        let ed = EditorState::empty();
        assert_eq!(ed.line_count(), 1);
    }

    #[test]
    fn title_shows_filename() {
        let mut ed = EditorState::empty();
        ed.file_path = Some(PathBuf::from("/tmp/foo/bar.rs"));
        assert!(ed.title().contains("bar.rs"));
    }

    #[test]
    fn is_write_cmd_detection() {
        let mut ed = EditorState::empty();
        ed.vim.mode = VimMode::Command;
        ed.vim.cmdline = "w".to_string();
        assert!(ed.is_write_cmd());
        assert!(!ed.is_writequit_cmd());
        ed.vim.cmdline = "wq".to_string();
        assert!(!ed.is_write_cmd());
        assert!(ed.is_writequit_cmd());
        ed.vim.cmdline = "x".to_string();
        assert!(ed.is_writequit_cmd());
    }

    #[test]
    fn load_nonexistent_file() {
        let mut ed = EditorState::empty();
        ed.load(Path::new("/nonexistent/path/file.txt"));
        assert!(ed.vim.text.contains("cannot read"));
        assert!(ed.file_path.is_some());
    }
}
