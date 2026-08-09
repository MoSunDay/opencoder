//! Key dispatch for the notepad view, split from `mod.rs` to respect the
//! 400-line-per-file cap.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::composer;
use crate::vim::{self, VimAction, VimMode};
use crate::notepad::editor::should_cycle_focus;
use crate::notepad::search;
use crate::notepad::terminal;
use crate::notepad::tree::TreeInput;
use crate::notepad::{Focus, NotepadOutcome, NotepadView};

/// Top-level key handler — delegates to the focused panel.
pub async fn handle_key(
    view: &mut NotepadView,
    k: KeyEvent,
    input: &mut String,
    cursor_idx: &mut usize,
) -> NotepadOutcome {
    if view.search.is_some() {
        return handle_search_key(view, k).await;
    }
    match view.focus {
        Focus::Tree => handle_tree_key(view, k),
        Focus::Editor => handle_editor_key(view, k),
        Focus::Terminal => handle_terminal_key(view, k, input, cursor_idx).await,
    }
}

// ── Tree ───────────────────────────────────────────────────────────────────

fn handle_tree_key(view: &mut NotepadView, k: KeyEvent) -> NotepadOutcome {
    if let Some(inp) = view.tree.input.take() {
        handle_tree_input(view, inp, k);
        return NotepadOutcome::Consumed;
    }
    match k.code {
        KeyCode::Esc => NotepadOutcome::Exit,
        KeyCode::Tab => { view.focus = Focus::Editor; NotepadOutcome::Consumed }
        KeyCode::Char('j') | KeyCode::Down => { view.tree.move_cursor(1); NotepadOutcome::Consumed }
        KeyCode::Char('k') | KeyCode::Up => { view.tree.move_cursor(-1); NotepadOutcome::Consumed }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => { open_or_expand(view); NotepadOutcome::Consumed }
        KeyCode::Left | KeyCode::Char('h') => { view.tree.collapse_dir(); NotepadOutcome::Consumed }
        KeyCode::Char('n') => { start_create(view); NotepadOutcome::Consumed }
        KeyCode::Char('d') => { start_delete(view); NotepadOutcome::Consumed }
        KeyCode::Char('H') => {
            view.tree_hidden = !view.tree_hidden;
            view.focus = if view.tree_hidden { Focus::Editor } else { Focus::Tree };
            NotepadOutcome::Consumed
        }
        KeyCode::Char('/') => { view.search = Some(search::SearchState::new()); NotepadOutcome::Consumed }
        _ => NotepadOutcome::Consumed,
    }
}

fn open_or_expand(view: &mut NotepadView) {
    let node = match view.tree.selected_node() {
        Some(n) => n.clone(),
        None => return,
    };
    if node.is_dir {
        view.tree.toggle_collapse();
    } else {
        view.editor.load(&node.path);
        view.focus = Focus::Editor;
    }
}

fn start_create(view: &mut NotepadView) {
    let parent = match view.tree.selected_node() {
        Some(n) if n.is_dir => n.path.clone(),
        Some(n) => n.path.parent().unwrap_or(&view.workdir).to_path_buf(),
        None => view.workdir.clone(),
    };
    view.tree.input = Some(TreeInput::Create { buf: String::new(), parent });
}

fn start_delete(view: &mut NotepadView) {
    if let Some(n) = view.tree.selected_node() {
        view.tree.input = Some(TreeInput::DeleteConfirm { path: n.path.clone() });
    }
}

fn handle_tree_input(view: &mut NotepadView, inp: TreeInput, k: KeyEvent) {
    match inp {
        TreeInput::Create { mut buf, parent } => match k.code {
            KeyCode::Esc => {}
            KeyCode::Enter if !buf.trim().is_empty() => {
                let _ = std::fs::write(parent.join(buf.trim()), "");
                view.tree.rebuild(&view.workdir);
            }
            KeyCode::Backspace => { buf.pop(); view.tree.input = Some(TreeInput::Create { buf, parent }); }
            KeyCode::Char(c) => { buf.push(c); view.tree.input = Some(TreeInput::Create { buf, parent }); }
            _ => { view.tree.input = Some(TreeInput::Create { buf, parent }); }
        },
        TreeInput::DeleteConfirm { path } => match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
                view.tree.rebuild(&view.workdir);
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {}
            _ => { view.tree.input = Some(TreeInput::DeleteConfirm { path }); }
        },
    }
}

// ── Editor ─────────────────────────────────────────────────────────────────

fn handle_editor_key(view: &mut NotepadView, k: KeyEvent) -> NotepadOutcome {
    let inner_w = editor_inner_width(view);
    if should_cycle_focus(&view.editor.vim, &k) {
        view.focus = Focus::Terminal;
        return NotepadOutcome::Consumed;
    }
    if view.editor.vim.mode == VimMode::Normal && k.code == KeyCode::Esc {
        return NotepadOutcome::Exit;
    }
    if view.editor.is_write_cmd() && k.code == KeyCode::Enter {
        let _ = view.editor.do_write();
        return NotepadOutcome::Consumed;
    }
    let action = vim::handle_vim_key(&mut view.editor.vim, k, inner_w, 2);
    if action == VimAction::Exit {
        if view.editor.is_modified() {
            let _ = view.editor.do_writequit();
        }
        view.focus = Focus::Tree;
    }
    NotepadOutcome::Consumed
}

fn editor_inner_width(view: &NotepadView) -> u16 {
    let (tw, _) = crossterm::terminal::size().unwrap_or((80, 24));
    let tree_w: u16 = if view.tree_hidden { 0 } else { 30 };
    tw.saturating_sub(tree_w + 4 + 2)
}

// ── Terminal ───────────────────────────────────────────────────────────────

async fn handle_terminal_key(
    view: &mut NotepadView,
    k: KeyEvent,
    input: &mut String,
    cursor_idx: &mut usize,
) -> NotepadOutcome {
    match k.code {
        KeyCode::Esc => NotepadOutcome::Exit,
        KeyCode::Tab => { view.focus = Focus::Tree; NotepadOutcome::Consumed }
        KeyCode::Up => { view.terminal.scroll_up(); NotepadOutcome::Consumed }
        KeyCode::Down => { view.terminal.scroll_down(); NotepadOutcome::Consumed }
        KeyCode::Enter => {
            let cmd = input.clone();
            input.clear();
            *cursor_idx = 0;
            if !cmd.trim().is_empty() {
                view.terminal.push_command(&cmd);
                let out = terminal::run_command(&cmd, &view.workdir).await;
                view.terminal.push_output(&out);
            }
            NotepadOutcome::Consumed
        }
        KeyCode::Backspace => {
            if let Some((t, c)) = composer::backspace(input, *cursor_idx) {
                *input = t;
                *cursor_idx = c;
            }
            NotepadOutcome::Consumed
        }
        KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
            let (t, c) = composer::insert_char(input, *cursor_idx, c);
            *input = t;
            *cursor_idx = c;
            NotepadOutcome::Consumed
        }
        _ => NotepadOutcome::Consumed,
    }
}

// ── Search ─────────────────────────────────────────────────────────────────

async fn handle_search_key(view: &mut NotepadView, k: KeyEvent) -> NotepadOutcome {
    let s = match view.search.as_mut() {
        Some(s) => s,
        None => return NotepadOutcome::Consumed,
    };
    if s.editing {
        match k.code {
            KeyCode::Esc => { view.search = None; }
            KeyCode::Enter => {
                let q = s.query.clone();
                s.status.clear();
                match search::search(&q, &view.workdir).await {
                    Ok(hits) => { s.results = hits; s.editing = false; s.selected = 0; s.scroll = 0; }
                    Err(e) => s.status = e,
                }
            }
            KeyCode::Backspace => { s.query.pop(); }
            KeyCode::Char(c) => { s.query.push(c); }
            _ => {}
        }
        return NotepadOutcome::Consumed;
    }
    match k.code {
        KeyCode::Esc => { view.search = None; }
        KeyCode::Char('j') | KeyCode::Down => { s.move_cursor(1); }
        KeyCode::Char('k') | KeyCode::Up => { s.move_cursor(-1); }
        KeyCode::Enter => {
            if let Some(hit) = s.selected_hit().cloned() {
                open_search_hit(view, hit);
            }
        }
        KeyCode::Char('/') => { s.editing = true; s.query.clear(); s.results.clear(); }
        _ => {}
    }
    NotepadOutcome::Consumed
}

fn open_search_hit(view: &mut NotepadView, hit: search::SearchHit) {
    view.editor.load(&hit.path);
    let target = hit.line_no.saturating_sub(1);
    let mut char_idx = 0usize;
    let mut line = 0usize;
    for ch in view.editor.vim.text.chars() {
        if line == target {
            break;
        }
        if ch == '\n' {
            line += 1;
        }
        char_idx += 1;
    }
    view.editor.vim.cursor = char_idx;
    view.editor.vim.mode = VimMode::Normal;
    view.editor.scroll = target as u16;
    view.focus = Focus::Editor;
    view.search = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notepad::NotepadView;

    fn key(code: KeyCode) -> KeyEvent { KeyEvent::new(code, KeyModifiers::NONE) }

    fn make_view(dir: &std::path::Path) -> NotepadView {
        std::fs::write(dir.join("a.txt"), "hello\nworld").unwrap();
        NotepadView::new(dir.to_path_buf())
    }

    #[test]
    fn tree_tab_cycles_to_editor() {
        let d = tempfile::tempdir().unwrap();
    let mut v = make_view(d.path());
        v.focus = Focus::Tree;
        assert_eq!(handle_tree_key(&mut v, key(KeyCode::Tab)), NotepadOutcome::Consumed);
        assert_eq!(v.focus, Focus::Editor);
    }

    #[test]
    fn tree_esc_exits() {
        let d = tempfile::tempdir().unwrap();
    let mut v = make_view(d.path());
        assert_eq!(handle_tree_key(&mut v, key(KeyCode::Esc)), NotepadOutcome::Exit);
    }

    #[test]
    fn editor_esc_normal_exits() {
        let d = tempfile::tempdir().unwrap();
    let mut v = make_view(d.path());
        v.focus = Focus::Editor;
        v.editor.vim.mode = VimMode::Normal;
        assert_eq!(handle_editor_key(&mut v, key(KeyCode::Esc)), NotepadOutcome::Exit);
    }

    #[tokio::test]
    async fn terminal_esc_exits() {
        let d = tempfile::tempdir().unwrap();
    let mut v = make_view(d.path());
        v.focus = Focus::Terminal;
        let mut input = String::new();
        let mut cur = 0;
        assert_eq!(
            handle_terminal_key(&mut v, key(KeyCode::Esc), &mut input, &mut cur).await,
            NotepadOutcome::Exit
        );
    }

    #[tokio::test]
    async fn terminal_runs_command() {
        let d = tempfile::tempdir().unwrap();
    let mut v = make_view(d.path());
        v.focus = Focus::Terminal;
        let mut input = String::from("echo hi");
        let mut cur = 7;
        handle_terminal_key(&mut v, key(KeyCode::Enter), &mut input, &mut cur).await;
        assert!(v.terminal.lines.iter().any(|l| l.text.contains("hi")));
        assert!(input.is_empty());
    }

    #[tokio::test]
    async fn search_finds_and_opens() {
        let d = tempfile::tempdir().unwrap();
    let mut v = make_view(d.path());
        v.search = Some(search::SearchState::new());
        // type query
        for c in "hello".chars() {
            handle_search_key(&mut v, key(KeyCode::Char(c))).await;
        }
        // execute search
        handle_search_key(&mut v, key(KeyCode::Enter)).await;
        let s = v.search.as_ref().unwrap();
        assert!(!s.results.is_empty());
        assert!(!s.editing);
        // open first result
        handle_search_key(&mut v, key(KeyCode::Enter)).await;
        assert_eq!(v.focus, Focus::Editor);
        assert!(v.search.is_none());
    }
}
