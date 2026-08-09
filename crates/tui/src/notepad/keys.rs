//! Key dispatch for the notepad view, split from `mod.rs` to respect the
//! 400-line-per-file cap.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::notepad::editor::should_cycle_focus;
use crate::notepad::search;
use crate::notepad::tree::TreeInput;
use crate::notepad::{Focus, NotepadOutcome, NotepadView};
use crate::vim::{self, VimAction, VimMode};

/// Top-level key handler — delegates to the focused panel.
pub async fn handle_key(view: &mut NotepadView, k: KeyEvent) -> NotepadOutcome {
    if view.search.is_some() {
        return handle_search_key(view, k).await;
    }
    match view.focus {
        Focus::Tree => handle_tree_key(view, k),
        Focus::Editor => handle_editor_key(view, k),
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
        KeyCode::Tab => {
            view.focus = Focus::Editor;
            NotepadOutcome::Consumed
        }
        KeyCode::Char('j') | KeyCode::Down => {
            view.tree.move_cursor(1);
            NotepadOutcome::Consumed
        }
        KeyCode::Char('k') | KeyCode::Up => {
            view.tree.move_cursor(-1);
            NotepadOutcome::Consumed
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            open_or_expand(view);
            NotepadOutcome::Consumed
        }
        KeyCode::Left | KeyCode::Char('h') => {
            view.tree.collapse_dir();
            NotepadOutcome::Consumed
        }
        KeyCode::Char('n') => {
            start_create(view);
            NotepadOutcome::Consumed
        }
        KeyCode::Char('d') => {
            start_delete(view);
            NotepadOutcome::Consumed
        }
        KeyCode::Char('H') => {
            view.tree_hidden = !view.tree_hidden;
            if view.tree_hidden {
                view.focus = Focus::Editor;
            }
            NotepadOutcome::Consumed
        }
        KeyCode::Char('/') => {
            view.search = Some(search::SearchState::new());
            NotepadOutcome::Consumed
        }
        _ => NotepadOutcome::Consumed,
    }
}

fn open_or_expand(view: &mut NotepadView) {
    if let Some(node) = view.tree.selected_node() {
        if node.is_dir {
            view.tree.toggle_collapse();
        } else {
            view.editor.load(&node.path);
            view.focus = Focus::Editor;
        }
    }
}

fn start_create(view: &mut NotepadView) {
    if let Some(node) = view.tree.selected_node() {
        let parent = if node.is_dir {
            node.path.clone()
        } else if let Some(p) = node.path.parent() {
            p.to_path_buf()
        } else {
            node.path.clone()
        };
        view.tree.input = Some(TreeInput::Create {
            buf: String::new(),
            parent,
        });
    }
}

fn start_delete(view: &mut NotepadView) {
    if let Some(node) = view.tree.selected_node().cloned() {
        view.tree.input = Some(TreeInput::DeleteConfirm { path: node.path });
    }
}

fn handle_tree_input(view: &mut NotepadView, inp: TreeInput, k: KeyEvent) {
    match inp {
        TreeInput::Create { mut buf, parent } => match k.code {
            KeyCode::Esc => {}
            KeyCode::Enter => {
                if !buf.trim().is_empty() {
                    let p = parent.join(&buf);
                    if let Some(par) = p.parent() {
                        let _ = std::fs::create_dir_all(par);
                    }
                    let _ = std::fs::write(&p, "");
                    view.tree.rebuild(&view.workdir);
                }
            }
            KeyCode::Backspace => {
                buf.pop();
                view.tree.input = Some(TreeInput::Create { buf, parent });
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                buf.push(c);
                view.tree.input = Some(TreeInput::Create { buf, parent });
            }
            _ => {}
        },
        TreeInput::DeleteConfirm { path } => match k.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if path.is_dir() {
                    let _ = std::fs::remove_dir_all(&path);
                } else {
                    let _ = std::fs::remove_file(&path);
                }
                view.tree.rebuild(&view.workdir);
            }
            _ => {}
        },
    }
}

// ── Editor ─────────────────────────────────────────────────────────────────

fn handle_editor_key(view: &mut NotepadView, k: KeyEvent) -> NotepadOutcome {
    let inner_w = editor_inner_width(view);
    let inner_h = editor_inner_height(view);

    // Tab in Normal mode cycles back to Tree.
    if should_cycle_focus(&view.editor.vim, &k) {
        view.focus = Focus::Tree;
        return NotepadOutcome::Consumed;
    }

    if view.editor.vim.mode == VimMode::Normal && k.code == KeyCode::Esc {
        return NotepadOutcome::Exit;
    }

    // Intercept :w / :wq / :x before the vim engine treats them as unknown.
    if k.code == KeyCode::Enter && view.editor.is_write_cmd() {
        let _ = view.editor.do_write();
        return NotepadOutcome::Consumed;
    }
    if k.code == KeyCode::Enter && view.editor.is_writequit_cmd() {
        let _ = view.editor.do_writequit();
        view.focus = Focus::Tree;
        return NotepadOutcome::Consumed;
    }

    // Intercept :e / :edit {path} to open a file.
    if k.code == KeyCode::Enter {
        if let Some(arg) = view.editor.edit_cmd_path() {
            view.editor.do_edit(&view.workdir, &arg);
            view.editor.ensure_cursor_visible(inner_h);
            return NotepadOutcome::Consumed;
        }
    }

    // Page-scroll keys in Normal mode (vim-style Ctrl-D/U/F/B, PageDown/PageUp).
    if view.editor.vim.mode == VimMode::Normal
        && try_page_scroll(view, &k, inner_h)
    {
        return NotepadOutcome::Consumed;
    }

    let action = vim::handle_vim_key(&mut view.editor.vim, k, inner_w, 2);
    view.editor.ensure_cursor_visible(inner_h);
    if action == VimAction::Exit {
        if view.editor.is_modified() {
            let _ = view.editor.do_writequit();
        }
        view.focus = Focus::Tree;
        return NotepadOutcome::Consumed;
    }
    NotepadOutcome::Consumed
}

/// Handle vim-style page-scroll keys in Normal mode. Returns `true` if handled.
fn try_page_scroll(view: &mut NotepadView, k: &KeyEvent, inner_h: u16) -> bool {
    let h = inner_h as usize;
    if h == 0 {
        return false;
    }
    let half = (h / 2).max(1);
    let full = h;
    let handled = match (k.code, k.modifiers) {
        (KeyCode::Char('d'), m) if m.contains(KeyModifiers::CONTROL) => {
            view.editor.page_down(half);
            true
        }
        (KeyCode::Char('u'), m) if m.contains(KeyModifiers::CONTROL) => {
            view.editor.page_up(half);
            true
        }
        (KeyCode::Char('f'), m) if m.contains(KeyModifiers::CONTROL) => {
            view.editor.page_down(full);
            true
        }
        (KeyCode::Char('b'), m) if m.contains(KeyModifiers::CONTROL) => {
            view.editor.page_up(full);
            true
        }
        (KeyCode::PageDown, _) => {
            view.editor.page_down(full);
            true
        }
        (KeyCode::PageUp, _) => {
            view.editor.page_up(full);
            true
        }
        _ => false,
    };
    if handled {
        view.editor.ensure_cursor_visible(inner_h);
    }
    handled
}

fn editor_inner_width(view: &NotepadView) -> u16 {
    let (tw, _) = crossterm::terminal::size().unwrap_or((80, 24));
    let tree_w: u16 = if view.tree_hidden { 0 } else { 30 };
    tw.saturating_sub(tree_w + 4 + 2)
}

fn editor_inner_height(view: &NotepadView) -> u16 {
    let (_, th) = crossterm::terminal::size().unwrap_or((80, 24));
    let max_height = th.saturating_sub(super::MIN_BOTTOM + 1);
    let clamped = view.height.clamp(5, max_height.max(5));
    clamped.saturating_sub(2)
}

// ── Search ───────────────────────────────────────────────────────────────

async fn handle_search_key(view: &mut NotepadView, k: KeyEvent) -> NotepadOutcome {
    let s = view.search.as_mut().unwrap();
    if s.editing {
        match k.code {
            KeyCode::Esc => {
                view.search = None;
            }
            KeyCode::Enter => {
                s.editing = false;
                let q = s.query.clone();
                let wd = view.workdir.clone();
                s.results = search::search(&q, &wd).await.unwrap_or_default();
                s.status = format!("{} results", s.results.len());
                s.selected = 0;
            }
            KeyCode::Backspace => {
                s.query.pop();
            }
            KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => {
                s.query.push(c);
            }
            _ => {}
        }
        return NotepadOutcome::Consumed;
    }
    match k.code {
        KeyCode::Esc => {
            view.search = None;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            view.search.as_mut().unwrap().move_cursor(1);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            view.search.as_mut().unwrap().move_cursor(-1);
        }
        KeyCode::Enter => {
            open_search_hit(view);
        }
        KeyCode::Char('/') => {
            view.search.as_mut().unwrap().editing = true;
        }
        _ => {}
    }
    NotepadOutcome::Consumed
}

fn open_search_hit(view: &mut NotepadView) {
    let hit = view.search.as_ref().and_then(|s| s.selected_hit()).cloned();
    if let Some(hit) = hit {
        view.editor.load(&hit.path);
        let target = hit.line_no.saturating_sub(1) as u16;
        view.editor.vim.mode = VimMode::Normal;
        let mut line = 0usize;
        let mut char_idx = 0usize;
        for ch in view.editor.vim.text.chars() {
            if line == target as usize {
                break;
            }
            if ch == '\n' {
                line += 1;
            }
            char_idx += 1;
        }
        view.editor.vim.cursor = char_idx;
        view.editor.scroll = target;
        view.focus = Focus::Editor;
        view.search = None;
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notepad::NotepadView;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn make_view(dir: &std::path::Path) -> NotepadView {
        NotepadView::new(dir.to_path_buf())
    }

    #[test]
    fn tree_esc_exits() {
        let d = tempfile::tempdir().unwrap();
        let mut v = make_view(d.path());
        assert_eq!(handle_tree_key(&mut v, key(KeyCode::Esc)), NotepadOutcome::Exit);
    }

    #[test]
    fn tree_tab_cycles_to_editor() {
        let d = tempfile::tempdir().unwrap();
        let mut v = make_view(d.path());
        assert_eq!(
            handle_tree_key(&mut v, key(KeyCode::Tab)),
            NotepadOutcome::Consumed
        );
        assert_eq!(v.focus, Focus::Editor);
    }

    #[test]
    fn tree_jk_move_cursor() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.txt"), "x").unwrap();
        std::fs::write(d.path().join("b.txt"), "y").unwrap();
        let mut v = make_view(d.path());
        handle_tree_key(&mut v, key(KeyCode::Char('j')));
        assert_eq!(handle_tree_key(&mut v, key(KeyCode::Char('k'))), NotepadOutcome::Consumed);
    }

    #[test]
    fn tree_enter_opens_file() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.txt"), "hello").unwrap();
        let mut v = make_view(d.path());
        handle_tree_key(&mut v, key(KeyCode::Enter));
        assert!(v.editor.file_path.is_some());
        assert_eq!(v.focus, Focus::Editor);
    }

    #[test]
    fn editor_tab_cycles_to_tree() {
        let d = tempfile::tempdir().unwrap();
        let mut v = make_view(d.path());
        v.focus = Focus::Editor;
        v.editor.vim.mode = VimMode::Normal;
        assert_eq!(
            handle_editor_key(&mut v, key(KeyCode::Tab)),
            NotepadOutcome::Consumed
        );
        assert_eq!(v.focus, Focus::Tree);
    }

    #[test]
    fn editor_esc_exits() {
        let d = tempfile::tempdir().unwrap();
        let mut v = make_view(d.path());
        v.focus = Focus::Editor;
        v.editor.vim.mode = VimMode::Normal;
        assert_eq!(
            handle_editor_key(&mut v, key(KeyCode::Esc)),
            NotepadOutcome::Exit
        );
    }

    #[test]
    fn tree_toggle_hide() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.txt"), "x").unwrap();
        let mut v = make_view(d.path());
        handle_tree_key(&mut v, key(KeyCode::Char('H')));
        assert!(v.tree_hidden);
        assert_eq!(v.focus, Focus::Editor);
    }

    #[test]
    fn tree_slash_opens_search() {
        let d = tempfile::tempdir().unwrap();
        let mut v = make_view(d.path());
        handle_tree_key(&mut v, key(KeyCode::Char('/')));
        assert!(v.search.is_some());
    }

    #[tokio::test]
    async fn search_finds_and_opens() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.txt"), "hello world").unwrap();
        let mut v = make_view(d.path());
        v.search = Some(search::SearchState::new());
        for c in "hello".chars() {
            handle_search_key(&mut v, key(KeyCode::Char(c))).await;
        }
        handle_search_key(&mut v, key(KeyCode::Enter)).await;
        let s = v.search.as_ref().unwrap();
        assert!(!s.results.is_empty());
        assert!(!s.editing);
        handle_search_key(&mut v, key(KeyCode::Enter)).await;
        assert_eq!(v.focus, Focus::Editor);
        assert!(v.search.is_none());
    }
}
