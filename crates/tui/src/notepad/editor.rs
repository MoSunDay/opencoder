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

use super::editor_layout::EditorLayout;

/// Screen dimensions available to editor text after borders and line numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditorViewport {
    pub text_width: u16,
    pub height: u16,
}

impl EditorViewport {
    pub fn for_area(area: Rect, line_count: usize) -> Self {
        let inner_width = area.width.saturating_sub(2);
        Self {
            text_width: inner_width.saturating_sub(gutter_width(line_count)),
            height: area.height.saturating_sub(2),
        }
    }
}

/// Editor state wrapping the shared vim engine.
#[derive(Clone, Debug)]
pub struct EditorState {
    pub vim: VimState,
    pub file_path: Option<PathBuf>,
    /// Vertical scroll offset (visual rows).
    pub scroll: usize,
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
            .unwrap_or_else(|_| format!("# (cannot read {})\n", path.display()));
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
        self.vim.mode == VimMode::Command && matches!(self.vim.cmdline.trim(), "wq" | "x")
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

    /// If in Command mode, parse `:e {path}` / `:edit {path}` and return the
    /// path argument. Returns `Some("")` for bare `:e` / `:edit` (reopen
    /// current file). Returns `None` if not an edit command.
    pub fn edit_cmd_path(&self) -> Option<String> {
        if self.vim.mode != VimMode::Command {
            return None;
        }
        let trimmed = self.vim.cmdline.trim();
        let (cmd, arg) = match trimmed.split_once(char::is_whitespace) {
            Some((c, a)) => (c, a.trim()),
            None => (trimmed, ""),
        };
        match cmd {
            "e" | "edit" => Some(arg.to_string()),
            _ => None,
        }
    }

    /// Execute `:e {path}`: resolve `arg` relative to `workdir` and load
    /// the file into the editor.
    pub fn do_edit(&mut self, workdir: &Path, arg: &str) {
        let path = if arg.is_empty() {
            match &self.file_path {
                Some(p) => p.clone(),
                None => {
                    self.vim.status = "no file name".to_string();
                    self.vim.mode = VimMode::Normal;
                    self.vim.cmdline.clear();
                    return;
                }
            }
        } else {
            let p = Path::new(arg);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                workdir.join(arg)
            }
        };
        self.load(&path);
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

    /// Number of logical lines, including the empty line after a trailing
    /// newline, matching Vim's buffer model.
    pub fn line_count(&self) -> usize {
        self.vim.text.bytes().filter(|byte| *byte == b'\n').count() + 1
    }

    /// Adjust visual-row scroll so the cursor stays inside the viewport.
    pub fn ensure_cursor_visible(&mut self, viewport: EditorViewport) {
        if viewport.height == 0 {
            return;
        }
        let layout = EditorLayout::new(&self.vim.text, viewport.text_width);
        let vis_h = viewport.height as usize;
        let cur_row = layout.cursor_position(self.vim.cursor).0;
        let scroll = self.scroll;
        let h = vis_h.saturating_sub(1);
        if cur_row < scroll {
            self.scroll = cur_row;
        } else if cur_row > scroll + h {
            self.scroll = cur_row - h;
        }
        self.scroll = layout.clamp_scroll(self.scroll, vis_h);
    }

    /// Put the cursor's visual row at the top, clamped at the final page.
    pub fn scroll_cursor_to_top(&mut self, viewport: EditorViewport) {
        let layout = EditorLayout::new(&self.vim.text, viewport.text_width);
        let cursor_row = layout.cursor_position(self.vim.cursor).0;
        self.scroll = layout.clamp_scroll(cursor_row, viewport.height as usize);
    }

    /// Current cursor line number (0-indexed).
    pub fn cursor_line(&self) -> usize {
        let byte_off = char_byte_offset(&self.vim.text, self.vim.cursor);
        self.vim.text[..byte_off].matches('\n').count()
    }

    /// Move the cursor to the start of the given logical line (0-indexed,
    /// clamped to the last line).
    pub fn move_to_line(&mut self, line: usize) {
        let total = self.line_count();
        let target = line.min(total.saturating_sub(1));
        let mut current_line = 0usize;
        let mut char_idx = 0usize;
        for ch in self.vim.text.chars() {
            if current_line == target {
                break;
            }
            char_idx += 1;
            if ch == '\n' {
                current_line += 1;
            }
        }
        self.vim.cursor = char_idx;
    }

    /// Move the cursor down by `n` visual rows, preserving display column.
    pub fn page_down(&mut self, n: usize, text_width: u16) {
        let layout = EditorLayout::new(&self.vim.text, text_width);
        self.vim.cursor = layout.move_cursor_rows(self.vim.cursor, n as isize);
    }

    /// Move the cursor up by `n` visual rows, preserving display column.
    pub fn page_up(&mut self, n: usize, text_width: u16) {
        let layout = EditorLayout::new(&self.vim.text, text_width);
        self.vim.cursor = layout.move_cursor_rows(self.vim.cursor, -(n as isize));
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

    // Add mode label at bottom-left of the border.
    let mode_label = state.vim.mode_label();
    let block = block.title_bottom(
        ratatui::text::Line::from(format!(" {} ", mode_label))
            .alignment(ratatui::layout::Alignment::Left),
    );

    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let gutter_w = gutter_width(state.line_count());
    let text_w = inner.width.saturating_sub(gutter_w);
    let layout = EditorLayout::new(&state.vim.text, text_w);
    let vis_h = inner.height as usize;
    let scroll = layout.clamp_scroll(state.scroll, vis_h);
    let total = layout.rows().len();

    let start = scroll.min(total);
    let end = (start + vis_h).min(total);

    let mut row_lines: Vec<Line> = Vec::new();
    for row in &layout.rows()[start..end] {
        let num_str = match row.line_number {
            Some(line_no) => format!(
                "{:>width$}  ",
                line_no,
                width = gutter_w.saturating_sub(2) as usize
            ),
            None => " ".repeat(gutter_w as usize),
        };
        let num_span = Span::styled(num_str, Style::default().fg(theme::subtle()));
        let content_span = Span::raw(layout.row_text(*row));
        row_lines.push(Line::from(vec![num_span, content_span]));
    }

    let para = Paragraph::new(row_lines).scroll((0, 0));
    f.render_widget(para, inner);

    // Position hardware cursor inside the editor.
    set_editor_cursor(f, inner, state, &layout, scroll, gutter_w);
}

/// Compute the cursor's terminal position and place it.
fn set_editor_cursor(
    f: &mut Frame,
    inner: Rect,
    state: &EditorState,
    layout: &EditorLayout,
    scroll: usize,
    gutter_w: u16,
) {
    if state.vim.mode == VimMode::Command || state.vim.mode == VimMode::Search {
        // Don't fight the cmdline — just show the cursor at the end of the
        // status line. It's approximate but avoids double-cursor artifacts.
        return;
    }
    let (row, col) = layout.cursor_position(state.vim.cursor);
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

pub(crate) fn gutter_width(line_count: usize) -> u16 {
    line_count.to_string().len().saturating_add(2) as u16
}

/// Plain text of every visual row of `text` soft-wrapped at `width`, in
/// order — the shared source for copy-mode's clean notepad view, so the
/// wrap model can never disagree with the decorated renderer. Pure: derives
/// from the buffer text only, no editor state.
pub fn row_texts(text: &str, width: u16) -> Vec<String> {
    let layout = EditorLayout::new(text, width.max(1));
    let rows = layout.rows();
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let mut s: String = chars[row.range.start..row.range.end].iter().collect();
        // The wrap ranges skip the hard newlines that terminate logical
        // lines; re-insert them so the concatenated rows reconstruct the
        // buffer exactly (renderers trim them back off — see copy_mode).
        if let Some(next) = rows.get(i + 1) {
            s.extend(chars[row.range.end..next.range.start].iter());
        }
        out.push(s);
    }
    out
}

/// Check if this key, when in Normal mode, is a plain focus-cycle key that
/// should NOT be sent to the vim engine.
pub fn is_focus_cycle_key(k: &crossterm::event::KeyEvent) -> bool {
    use crossterm::event::KeyCode;
    k.code == KeyCode::Tab && state_normal_or(k)
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
    fn row_texts_round_trips_and_wraps_in_order() {
        // Concatenated rows reconstruct the buffer exactly (no chars lost to
        // the wrap), narrow width forces multiple visual rows, and rows come
        // back in order.
        let rows = row_texts("alpha beta gamma\ndelta", 6);
        assert_eq!(rows.concat(), "alpha beta gamma\ndelta");
        assert!(rows.len() > 3, "must wrap: {rows:?}");
        assert_eq!(rows[0], "alpha ");

        // Empty buffer yields a single empty row (matches the wrap model).
        assert_eq!(row_texts("", 10), vec![String::new()]);
    }

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
    fn line_count_includes_empty_line_after_trailing_newline() {
        let mut ed = EditorState::empty();
        ed.vim.text = "a\nb\n".to_string();
        assert_eq!(ed.line_count(), 3);
    }

    #[test]
    fn viewport_accounts_for_dynamic_gutter_width() {
        let two_digit = EditorViewport::for_area(Rect::new(0, 0, 50, 20), 99);
        let three_digit = EditorViewport::for_area(Rect::new(0, 0, 50, 20), 100);
        assert_eq!(two_digit, viewport(44, 18));
        assert_eq!(three_digit, viewport(43, 18));
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

    fn make_tall_editor(lines: usize) -> EditorState {
        let text: String = (1..=lines)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let mut ed = EditorState::empty();
        ed.vim = VimState::new(text);
        ed.vim.mode = VimMode::Normal;
        ed.vim.cursor = 0;
        ed
    }

    fn viewport(width: u16, height: u16) -> EditorViewport {
        EditorViewport {
            text_width: width,
            height,
        }
    }

    #[test]
    fn ensure_cursor_no_change_when_visible() {
        let mut ed = make_tall_editor(20);
        ed.scroll = 0;
        ed.ensure_cursor_visible(viewport(40, 10));
        assert_eq!(ed.scroll, 0);
    }

    #[test]
    fn ensure_cursor_scrolls_down_when_past_bottom() {
        let mut ed = make_tall_editor(20);
        ed.move_to_line(15);
        ed.scroll = 0;
        ed.ensure_cursor_visible(viewport(40, 10));
        // vis_h=10, h=9, cur_row=15 → scroll = 15-9 = 6
        assert_eq!(ed.scroll, 6);
    }

    #[test]
    fn ensure_cursor_scrolls_up_when_above_top() {
        let mut ed = make_tall_editor(20);
        ed.move_to_line(3);
        ed.scroll = 8;
        ed.ensure_cursor_visible(viewport(40, 10));
        assert_eq!(ed.scroll, 3);
    }

    #[test]
    fn ensure_cursor_clamps_to_last_page() {
        let mut ed = make_tall_editor(20);
        ed.move_to_line(19);
        ed.scroll = 0;
        ed.ensure_cursor_visible(viewport(40, 10));
        // cur_row=19, h=9 → raw scroll=10, but max_scroll=20-10=10, so 10
        assert_eq!(ed.scroll, 10);
    }

    #[test]
    fn cursor_line_correct_at_end_of_text() {
        let mut ed = EditorState::empty();
        ed.vim = VimState::new("a\nb\nc\n".to_string());
        ed.vim.cursor = 4; // at 'c'
        assert_eq!(ed.cursor_line(), 2);
        ed.vim.cursor = 6; // past trailing newline (end of buffer)
        assert_eq!(ed.cursor_line(), 3);
    }

    #[test]
    fn move_to_line_clamps() {
        let mut ed = make_tall_editor(5);
        ed.move_to_line(100);
        assert_eq!(ed.cursor_line(), 4); // clamped to last line
    }

    #[test]
    fn page_down_moves_half() {
        let mut ed = make_tall_editor(30);
        assert_eq!(ed.cursor_line(), 0);
        ed.page_down(5, 40);
        assert_eq!(ed.cursor_line(), 5);
    }

    #[test]
    fn page_up_clamps_to_zero() {
        let mut ed = make_tall_editor(30);
        ed.move_to_line(2);
        ed.page_up(10, 40);
        assert_eq!(ed.cursor_line(), 0);
    }

    #[test]
    fn edit_cmd_path_parses_e_with_arg() {
        let mut ed = EditorState::empty();
        ed.vim.mode = VimMode::Command;
        ed.vim.cmdline = "e foo.txt".to_string();
        assert_eq!(ed.edit_cmd_path(), Some("foo.txt".to_string()));
    }

    #[test]
    fn edit_cmd_path_parses_edit_with_arg() {
        let mut ed = EditorState::empty();
        ed.vim.mode = VimMode::Command;
        ed.vim.cmdline = "edit bar/baz.rs".to_string();
        assert_eq!(ed.edit_cmd_path(), Some("bar/baz.rs".to_string()));
    }

    #[test]
    fn edit_cmd_path_bare_e_returns_empty() {
        let mut ed = EditorState::empty();
        ed.vim.mode = VimMode::Command;
        ed.vim.cmdline = "e".to_string();
        assert_eq!(ed.edit_cmd_path(), Some("".to_string()));
    }

    #[test]
    fn edit_cmd_path_rejects_non_edit() {
        let mut ed = EditorState::empty();
        ed.vim.mode = VimMode::Command;
        ed.vim.cmdline = "w".to_string();
        assert_eq!(ed.edit_cmd_path(), None);
        ed.vim.cmdline = "echo hi".to_string();
        assert_eq!(ed.edit_cmd_path(), None);
    }

    #[test]
    fn do_edit_loads_relative_path() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("target.txt"), "target content").unwrap();
        let mut ed = EditorState::empty();
        ed.do_edit(d.path(), "target.txt");
        assert_eq!(ed.vim.text, "target content");
        assert_eq!(ed.vim.mode, VimMode::Normal);
    }

    #[test]
    fn do_edit_reopens_current_file_when_no_arg() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("cur.txt");
        std::fs::write(&p, "v1").unwrap();
        let mut ed = EditorState::empty();
        ed.load(&p);
        ed.vim.text = "modified".to_string();
        // Reopen
        ed.vim.mode = VimMode::Command;
        ed.vim.cmdline = "e".to_string();
        let arg = ed.edit_cmd_path().unwrap();
        ed.do_edit(d.path(), &arg);
        assert_eq!(ed.vim.text, "v1"); // reloaded from disk
    }

    #[test]
    fn do_edit_sets_no_file_name_when_no_arg_and_no_path() {
        let d = tempfile::tempdir().unwrap();
        let mut ed = EditorState::empty();
        ed.vim.mode = VimMode::Command;
        ed.vim.cmdline = "e".to_string();
        ed.do_edit(d.path(), "");
        assert_eq!(ed.vim.status, "no file name");
        assert_eq!(ed.vim.mode, VimMode::Normal);
        assert!(ed.vim.cmdline.is_empty());
    }

    #[test]
    fn do_edit_loads_absolute_path() {
        let d = tempfile::tempdir().unwrap();
        let abs = d.path().join("abs.txt");
        fs::write(&abs, "abs content").unwrap();
        let mut ed = EditorState::empty();
        ed.do_edit(d.path(), abs.to_str().unwrap());
        assert_eq!(ed.vim.text, "abs content");
    }

    #[test]
    fn move_to_line_lands_on_exact_middle_line() {
        let mut ed = make_tall_editor(10);
        ed.move_to_line(4);
        assert_eq!(ed.cursor_line(), 4);
        // cursor sits at the start of the 5th logical line ("line 5")
        assert!(ed.vim.text[ed.vim.cursor..].starts_with("line 5"));
    }
}
