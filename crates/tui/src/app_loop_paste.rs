//! Paste / clipboard helpers extracted from `app_loop.rs` to keep that file
//! under the 800-line iteration cap. This is a pure move: the logic,
//! signatures and doc comments are unchanged from their original inline
//! location. The `pub(crate)` items are re-exported from `app_loop.rs`
//! (`pub(crate) use app_loop_paste::{...}`), so all existing call sites —
//! `app.rs` and the test modules that do `use super::*` — keep resolving
//! unchanged.

use std::path::Path;

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::LoopFlow;
use crate::app_helpers::paste_payload;
use crate::chat::ChatView;
use crate::command::CommandMenu;
use crate::composer;
use crate::model_menu::ModelMenu;
use crate::theme;

/// Paste an image (screenshot bitmap) from the system clipboard into the
/// composer's `pending_images`. Triggered by `Ctrl+V`. Replaces the former
/// `/clip` slash command. The blocking `arboard` clipboard read runs on a
/// background thread so it can't stall the async event loop.
pub(crate) async fn paste_clipboard_image(
    chat: &mut ChatView,
    pending_images: &mut Vec<(String, String)>,
) -> LoopFlow {
    match tokio::task::spawn_blocking(crate::clipboard::clipboard_image_data_uri).await {
        Ok(Some(data_uri)) => {
            let n = pending_images.len() + 1;
            pending_images.push((data_uri, "clipboard.png".to_string()));
            chat.push_marker(Line::from(Span::styled(
                format!("\u{1f4ce} pasted image from clipboard ({n} attached)"),
                Style::default().fg(theme::ok_color()),
            )));
        }
        Ok(None) => {
            chat.push_marker(Line::from(Span::styled(
                "[clip] no image in clipboard",
                Style::default().fg(theme::warn_color()),
            )));
        }
        Err(e) => {
            chat.push_marker(Line::from(Span::styled(
                format!("[clip] clipboard read failed: {e}"),
                Style::default().fg(theme::err_color()),
            )));
        }
    }
    LoopFlow::Proceed
}

/// Like [`paste_clipboard_image`] but silent on failure. Used when an empty
/// bracketed-paste arrives — the terminal swallowed Ctrl+V because the
/// clipboard holds an image, not text. Success still shows the 📎 marker;
/// "no image" or clipboard errors are quietly ignored so the user is never
/// disturbed by an empty paste.
pub(crate) async fn paste_clipboard_image_silent(
    chat: &mut ChatView,
    pending_images: &mut Vec<(String, String)>,
) {
    if let Ok(Some(data_uri)) =
        tokio::task::spawn_blocking(crate::clipboard::clipboard_image_data_uri).await
    {
        pending_images.push((data_uri, "clipboard.png".to_string()));
        push_attach_marker(chat, pending_images.len(), "pasted image from clipboard");
    }
}

/// Push a green `📎 {label} ({n} attached)` marker into the chat stream.
fn push_attach_marker(chat: &mut ChatView, n: usize, label: &str) {
    chat.push_marker(Line::from(Span::styled(
        format!("\u{1f4ce} {label} ({n} attached)"),
        Style::default().fg(theme::ok_color()),
    )));
}

/// Route a paste payload by the same modal priority as key events: an open
/// popup owns the paste, so it never reaches the main input hidden behind it.
///
/// Mirrors [`Event::Key`](crossterm::event::Event::Key)'s priority chain:
/// - plan-edit / annotation editor open -> insert the payload verbatim at its
///   cursor via [`crate::plan_edit::PlanEdit::paste`] (never leaks to the
///   composer hidden underneath);
/// - notepad open -> insert verbatim when the vim editor has focus (search
///   box or tree focus swallows); either way the composer never sees it;
/// - task picker / cache-salt menu open -> modal isolation (no text fields),
///   swallow the paste;
/// - model menu open -> feed the trimmed payload to its focused field via
///   [`ModelMenu::paste`];
/// - command menu open -> append to its query and refilter via
///   [`CommandMenu::paste`];
/// - otherwise -> resolve a dragged file to its absolute path (or insert the
///   payload verbatim) into the main composer; image file paths are loaded
///   into `pending_images` as data URIs instead.
///
/// Returns [`LoopFlow::Redraw`] when a modal consumed the paste (the caller
/// re-renders), [`LoopFlow::Proceed`] when the main composer absorbed it
/// (`input`/`cursor_idx` updated in place). Never returns `Quit`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn route_paste(
    pasted: &str,
    plan_edit: &mut Option<crate::plan_edit::PlanEdit>,
    notepad: &mut Option<crate::notepad::NotepadView>,
    task_picker_open: bool,
    cache_salt_menu_open: bool,
    keymap_menu_open: bool,
    skill_toggle_menu_open: bool,
    model_menu: &mut Option<ModelMenu>,
    mcp_menu: &mut Option<crate::mcp_menu::McpMenu>,
    envs_menu: &mut Option<crate::envs_menu::EnvsMenu>,
    cli_menu: &mut Option<crate::cli_menu::CliMenu>,
    command_menu: &mut Option<CommandMenu>,
    question_menu: &mut Option<crate::question_menu::QuestionMenu>,
    input: &mut String,
    cursor_idx: &mut usize,
    pending_images: &mut Vec<(String, String)>,
    asm: &mut crate::image_chunk::Assembly,
    chat: &mut ChatView,
    workdir: &Path,
) -> LoopFlow {
    // Fullscreen vim editors own every paste: insert literally, never leak
    // to the composer underneath (paste events bypassed plan_edit/notepad,
    // silently landing in the hidden input buffer — annotation content loss).
    if let Some(pe) = plan_edit.as_mut() {
        pe.paste(pasted);
        return LoopFlow::Redraw;
    }
    if let Some(view) = notepad.as_mut() {
        if view.search.is_none() && view.focus == crate::notepad::Focus::Editor {
            crate::vim::paste_terminal(&mut view.editor.vim, pasted);
        }
        return LoopFlow::Redraw;
    }
    if task_picker_open || cache_salt_menu_open || keymap_menu_open || skill_toggle_menu_open {
        // No text fields here -- modal isolation: swallow the paste.
        return LoopFlow::Redraw;
    }
    // Modal fields never resolve drag-and-drop file paths; only strip the
    // trailing newline terminals append to pasted payloads.
    let trimmed = pasted.trim_end_matches(['\r', '\n']);
    if let Some(menu) = model_menu.as_mut() {
        menu.paste(trimmed);
        return LoopFlow::Redraw;
    }
    if let Some(menu) = mcp_menu.as_mut() {
        menu.paste(trimmed);
        return LoopFlow::Redraw;
    }
    if let Some(menu) = envs_menu.as_mut() {
        menu.paste(trimmed);
        return LoopFlow::Redraw;
    }
    if let Some(menu) = cli_menu.as_mut() {
        menu.paste(trimmed);
        return LoopFlow::Redraw;
    }
    if let Some(menu) = command_menu.as_mut() {
        menu.paste(trimmed);
        return LoopFlow::Redraw;
    }
    if let Some(menu) = question_menu.as_mut() {
        menu.paste_custom(trimmed);
        return LoopFlow::Redraw;
    }

    // Chunked image frames (ocimg protocol): tmux/SSH may truncate huge
    // single pastes, so scripts emit small self-delimiting frames the TUI
    // reassembles. Non-frame lines in the same paste fall through to the
    // composer as text.
    if trimmed.lines().any(|l| l.starts_with("ocimg ")) {
        let now = crate::image_chunk::now_ms();
        let mut leftover = String::new();
        for line in trimmed.lines() {
            use crate::image_chunk::FeedOutcome;
            match asm.feed_line(line, now) {
                FeedOutcome::NotFrame => {
                    if !leftover.is_empty() {
                        leftover.push('\n');
                    }
                    leftover.push_str(line);
                }
                FeedOutcome::Pending => {}
                FeedOutcome::Complete {
                    uri,
                    filename,
                    chunks,
                } => {
                    let label = format!("{filename} ({chunks} chunks)");
                    pending_images.push((uri, label.clone()));
                    push_attach_marker(chat, pending_images.len(), &label);
                }
                FeedOutcome::Warn { message } => {
                    chat.push_marker(Line::from(Span::styled(
                        message,
                        Style::default().fg(theme::warn_color()),
                    )));
                }
            }
        }
        if !leftover.is_empty() {
            let (new_input, new_idx) = composer::insert_str(input, *cursor_idx, &leftover);
            *input = new_input;
            *cursor_idx = new_idx;
        }
        return LoopFlow::Proceed;
    }

    // Inline data:image URI — attach verbatim (trailing newline already stripped).
    if let Some(filename) = crate::image_chunk::image_data_uri_filename(trimmed) {
        pending_images.push((trimmed.to_string(), filename.clone()));
        push_attach_marker(chat, pending_images.len(), &filename);
        return LoopFlow::Proceed;
    }

    // HTTP(S) URL pointing at an image — attach the URL with its filename.
    if let Some(filename) = crate::image_chunk::image_url_filename(trimmed) {
        pending_images.push((trimmed.to_string(), filename.clone()));
        push_attach_marker(chat, pending_images.len(), &filename);
        return LoopFlow::Proceed;
    }

    // Main composer: check if pasted content is an image file path.
    if let Some((data_uri, filename)) = crate::image_util::try_load_image(trimmed, workdir) {
        pending_images.push((data_uri, filename.clone()));
        push_attach_marker(chat, pending_images.len(), &filename);
        return LoopFlow::Proceed;
    }
    // Otherwise: drag a file in (or clipboard paste) arrives as one
    // atomic payload; resolve an existing file to its absolute path, else
    // insert verbatim.
    let payload = paste_payload(pasted, workdir);
    let (new_input, new_idx) = composer::insert_str(input, *cursor_idx, &payload);
    *input = new_input;
    *cursor_idx = new_idx;
    LoopFlow::Proceed
}

/// Handle one `Event::Paste` end-to-end (extracted from `app.rs` to keep
/// that file under the 800-line iteration cap). Empty pastes attempt a
/// silent clipboard-image read unless a fullscreen editor modal (plan_edit /
/// notepad) is open — then they are swallowed, no clipboard read behind the
/// overlay. Otherwise the paste is routed modal-first, mirroring `Event::Key`
/// priority (plan_edit / notepad before the popup menus). Returns `true`
/// when a modal consumed the paste (caller `continue`s — identical to the old
/// inline `Redraw` flow); `false` lets the loop fall through with `dirty`
/// already set.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_paste_event(
    pasted: &str,
    plan_edit: &mut Option<crate::plan_edit::PlanEdit>,
    notepad: &mut Option<crate::notepad::NotepadView>,
    task_picker_open: bool,
    cache_salt_menu_open: bool,
    keymap_menu_open: bool,
    skill_toggle_menu_open: bool,
    model_menu: &mut Option<ModelMenu>,
    mcp_menu: &mut Option<crate::mcp_menu::McpMenu>,
    envs_menu: &mut Option<crate::envs_menu::EnvsMenu>,
    cli_menu: &mut Option<crate::cli_menu::CliMenu>,
    command_menu: &mut Option<CommandMenu>,
    question_menu: &mut Option<crate::question_menu::QuestionMenu>,
    input: &mut String,
    cursor_idx: &mut usize,
    pending_images: &mut Vec<(String, String)>,
    asm: &mut crate::image_chunk::Assembly,
    chat: &mut ChatView,
    workdir: &Path,
) -> bool {
    let editor_open = plan_edit.is_some() || notepad.is_some();
    if pasted.trim().is_empty() && !editor_open {
        paste_clipboard_image_silent(chat, pending_images).await;
        return true;
    }
    matches!(
        route_paste(
            pasted,
            plan_edit,
            notepad,
            task_picker_open,
            cache_salt_menu_open,
            keymap_menu_open,
            skill_toggle_menu_open,
            model_menu,
            mcp_menu,
            envs_menu,
            cli_menu,
            command_menu,
            question_menu,
            input,
            cursor_idx,
            pending_images,
            asm,
            chat,
            workdir,
        ),
        LoopFlow::Redraw
    )
}
