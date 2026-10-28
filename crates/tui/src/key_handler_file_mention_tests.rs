//! @ is literal input and never opens a picker.

use super::*;

struct Ctx {
    input: String,
    cursor: usize,
    hist_idx: Option<usize>,
    scroll: u32,
    follow: bool,
    last_esc: Option<Instant>,
    skill_menu: Option<SkillMenu>,
    undo_state: crate::undo::UndoState,
    queue_scroll: u32,
}

impl Ctx {
    fn new(_workdir: &std::path::Path, input: &str) -> Self {
        Ctx {
            input: input.to_string(),
            cursor: input.chars().count(),
            hist_idx: None,
            scroll: 0,
            follow: true,
            last_esc: None,
            skill_menu: None,
            undo_state: crate::undo::init(input, input.chars().count()),
            queue_scroll: 0,
        }
    }

    fn key(&mut self, code: KeyCode, mods: KeyModifiers) -> KeyAction {
        let history: Vec<String> = Vec::new();
        handle_key(
            KeyEvent::new(code, mods),
            &crate::keymap::KeyBindings::from_config(&opencoder_core::Config::default()),
            &mut self.input,
            &mut self.cursor,
            &history,
            &mut self.hist_idx,
            false,
            false,
            "act",
            &mut self.scroll,
            &mut self.follow,
            &mut self.last_esc,
            &mut self.skill_menu,
            80,
            2,
            false,
            false,
            false,
            &mut self.undo_state,
            &mut self.queue_scroll,
            &mut None,
        )
    }
}

fn workdir_with_files() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("notes.md"), "n").unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
    dir
}

#[test]
fn at_is_literal_at_empty_token_boundary_and_inside_email() {
    let dir = workdir_with_files();
    for (before, after) in [("", "@"), ("read ", "read @"), ("a", "a@")] {
        let mut context = Ctx::new(dir.path(), before);
        assert!(matches!(
            context.key(KeyCode::Char('@'), KeyModifiers::NONE),
            KeyAction::None
        ));
        assert_eq!(context.input, after);
        assert_eq!(context.cursor, after.chars().count());
    }
}
#[test]
fn at_path_typing_submits_verbatim_without_a_picker() {
    let dir = workdir_with_files();
    let mut context = Ctx::new(dir.path(), "open ");
    for ch in "@notes.md".chars() {
        context.key(KeyCode::Char(ch), KeyModifiers::NONE);
    }
    let action = context.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(action,KeyAction::Submit(text) if text == "open @notes.md"));
    assert!(context.input.is_empty());
}
