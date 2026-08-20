//! Key-level tests for the `@` file-mention picker: opening at a token
//! start, NOT opening mid-token (emails), and the pick → token insertion
//! round-trip. Split out of `key_handler_tests.rs` to keep files under the
//! 800-line cap.

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
    file_menu: Option<crate::file_menu::FileMenu>,
    workdir: std::path::PathBuf,
}

impl Ctx {
    fn new(workdir: &std::path::Path, input: &str) -> Self {
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
            file_menu: None,
            workdir: workdir.to_path_buf(),
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
            "act",
            &mut self.scroll,
            &mut self.follow,
            &mut self.last_esc,
            &mut self.skill_menu,
            80,
            2,
            false,
            false,
            &mut self.undo_state,
            &mut self.queue_scroll,
            &mut self.file_menu,
            &self.workdir,
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
fn at_at_token_start_opens_menu_without_inserting() {
    let dir = workdir_with_files();
    let mut c = Ctx::new(dir.path(), "read ");
    let a = c.key(KeyCode::Char('@'), KeyModifiers::NONE);
    assert!(matches!(a, KeyAction::None));
    assert!(c.file_menu.is_some(), "menu must open");
    assert_eq!(c.input, "read ", "the '@' itself is consumed");
    assert!(c.file_menu.as_ref().unwrap().visible_count() >= 3);
}

#[test]
fn at_at_start_of_empty_input_opens_menu() {
    let dir = workdir_with_files();
    let mut c = Ctx::new(dir.path(), "");
    c.key(KeyCode::Char('@'), KeyModifiers::NONE);
    assert!(c.file_menu.is_some());
    assert!(c.input.is_empty());
}

#[test]
fn at_mid_token_never_opens_menu() {
    let dir = workdir_with_files();
    // Email: cursor sits after `a` — non-whitespace before the '@'.
    let mut c = Ctx::new(dir.path(), "a");
    c.key(KeyCode::Char('@'), KeyModifiers::NONE);
    assert!(
        c.file_menu.is_none(),
        "mid-token '@' must not open the menu"
    );
    assert_eq!(c.input, "a@", "the '@' inserts literally");
}

#[test]
fn filter_then_pick_pins_token_into_input() {
    let dir = workdir_with_files();
    let mut c = Ctx::new(dir.path(), "open ");
    c.key(KeyCode::Char('@'), KeyModifiers::NONE);
    // Type "notes" — filtered rows shrink to notes.md.
    for ch in "notes".chars() {
        c.key(KeyCode::Char(ch), KeyModifiers::NONE);
    }
    assert_eq!(c.file_menu.as_ref().unwrap().visible_count(), 1);
    let a = c.key(KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(a, KeyAction::None));
    assert!(c.file_menu.is_none(), "pick closes the menu");
    assert_eq!(c.input, "open @notes.md ");
    assert_eq!(c.cursor, c.input.chars().count(), "cursor trails the token");
}

#[test]
fn esc_closes_menu_and_leaves_input_untouched() {
    let dir = workdir_with_files();
    let mut c = Ctx::new(dir.path(), "see ");
    c.key(KeyCode::Char('@'), KeyModifiers::NONE);
    let a = c.key(KeyCode::Esc, KeyModifiers::NONE);
    assert!(matches!(a, KeyAction::None));
    assert!(c.file_menu.is_none());
    assert_eq!(c.input, "see ", "no '@' leaks after cancel");
}
