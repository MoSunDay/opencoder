//! `@` file-mention picker for the TUI composer.
//!
//! Typing `@` at a token start opens [`FileMenu`]: a dropdown anchored above
//! the composer listing workdir files (gitignore-aware), filtered live by
//! what follows. `Enter`/`Tab` pins the highlighted entry into the input as
//! an `@relative/path ` token (leading `@` re-emitted by the pick because
//! the trigger keystroke was consumed; trailing space) — the composer keeps
//! the short mention form, and submit-time expansion via
//! `opencoder_session::mention_resolve` rewrites it to an absolute path.
//! `Esc` closes. Mirrors the `/` command-picker (`command.rs`) structure so
//! `app.rs` stays a flat match. Picker-pinned and hand-typed `@path`
//! mentions ride the same expansion.

pub mod list;
pub mod state;
pub mod view;

pub use list::{collect_entries, FileEntry};
pub use state::{handle_file_key, FileMenu, FileOutcome};
pub use view::render_file_popup;
