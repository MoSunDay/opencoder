//! Frame rendering extracted from `app_loop.rs` to keep that file under the
//! 800-line cap. `render_frame` resolves the plan-edit/composer display state
//! and delegates to `crate::render::render`.

use std::io::Write;

use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::text::Line;

/// Lifetime (in animation ticks) of a transient mode flash shown in the
/// status line (plan-edit / mode-switch hints).
const MODE_FLASH_TICKS: u32 = 15;

/// Whether a transient flash started at `start` is still visible at `now`,
/// given a lifetime of `ticks` anim ticks. Uses wrapping subtraction so it
/// stays correct across the u32 wraparound of `anim_tick`.
pub(crate) fn flash_visible(start: u32, now: u32, ticks: u32) -> bool {
    now.wrapping_sub(start) < ticks
}

/// Transient mode-flash status text if still within its visibility window.
fn flash_status_text(mode_flash: &Option<(String, u32)>, anim_tick: u32) -> Option<&str> {
    mode_flash
        .as_ref()
        .and_then(|(t, s)| flash_visible(*s, anim_tick, MODE_FLASH_TICKS).then_some(t.as_str()))
}

/// Run one complete frame as a synchronized terminal update. Supporting
/// terminals (including tmux when its outer terminal advertises `Sync`) keep
/// showing the previous frame until `end` arrives, so users never see a
/// half-written mixture of old and new cells. Unknown private modes are
/// ignored by terminals without synchronized-update support.
///
/// `end` is attempted even when rendering fails: leaving mode 2026 enabled
/// would freeze all later terminal output. If both operations fail, retain the
/// render error as the primary cause and attach the cleanup failure as context.
fn synchronized_frame<S, T>(
    state: &mut S,
    begin: impl FnOnce(&mut S) -> std::io::Result<()>,
    render: impl FnOnce(&mut S) -> anyhow::Result<T>,
    end: impl FnOnce(&mut S) -> std::io::Result<()>,
) -> anyhow::Result<T> {
    begin(state)?;
    let render_result = render(state);
    let end_result = end(state);
    match (render_result, end_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) => Err(error.into()),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(end_error)) => Err(error.context(format!(
            "failed to end synchronized terminal update: {end_error}"
        ))),
    }
}

fn begin_synchronized_update(writer: &mut impl Write) -> std::io::Result<()> {
    // Keep begin in the same output stream as the frame. `Terminal::draw`
    // flushes after emitting its diff, so an eager flush here would add a
    // syscall without improving ordering: the terminal still parses begin
    // before every following cell update.
    crossterm::queue!(writer, BeginSynchronizedUpdate)
}

fn end_synchronized_update(writer: &mut impl Write) -> std::io::Result<()> {
    crossterm::execute!(writer, EndSynchronizedUpdate)
}

/// Render the full TUI frame in one call. Extracted from `run_app`'s render
/// block so that the plan-edit composer state, the transient mode-flash and
/// copy-status closures, and the 30-argument `render()` invocation live in a
/// single place; the call site in `app.rs` stays a thin one-liner. The plan
/// edit modal owns the composer (its text + cursor) when active, otherwise the
/// normal input line is shown.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_frame(
    terminal: &mut crate::render::Term,
    chat: &crate::chat::ChatView,
    plan_edit: &Option<crate::plan_edit::PlanEdit>,
    input: &str,
    cursor_idx: usize,
    title: &Line<'static>,
    running: bool,
    ctx: u64,
    sys: u64,
    compaction_threshold: u64,
    context_limit: u64,
    status: &str,
    steer_items: &[(i64, String)],
    queue_items: &[(i64, String)],
    scroll: &mut u32,
    follow: bool,
    queue_scroll: &mut u32,
    anim_tick: u32,
    now_ms: i64,
    mode_flash: &Option<(String, u32)>,
    skill_menu: Option<&crate::menu::SkillMenu>,
    task_picker: Option<&crate::task::TaskPicker>,
    command_menu: Option<&crate::command::CommandMenu>,
    model_menu: Option<&crate::model_menu::ModelMenu>,
    cache_salt_menu: Option<&crate::cache_salt_menu::CacheSaltMenu>,
    keymap_menu: Option<&crate::keymap_menu::KeymapMenu>,
    hits: &mut crate::render::MouseHits,
    viewport: &mut Option<crate::render_viewport::ViewportCache>,
    shift_held: bool,
    copy_mode: bool,
    pending_images: &[(String, String)],
    input_disabled: bool,
    tail_ms: u64,
    task_ms: u64,
    is_top_level: bool,
    ap_enabled: bool,
    display_mode: &str,
) -> anyhow::Result<()> {
    let plan_label = plan_edit.as_ref().map(|pe| pe.mode_label());
    let (render_input, render_cursor) = match plan_edit {
        Some(pe) => (pe.text(), pe.cursor()),
        None => (input, cursor_idx),
    };
    let plan_mode: Option<&str> = plan_label.as_deref();
    let edit_title: Option<&str> = plan_edit.as_ref().map(|pe| pe.title());
    synchronized_frame(
        terminal,
        |terminal| begin_synchronized_update(terminal.backend_mut()),
        |terminal| {
            crate::render::render(
                terminal,
                chat,
                render_input,
                render_cursor,
                title,
                running,
                ctx,
                sys,
                compaction_threshold,
                context_limit,
                status,
                steer_items,
                queue_items,
                scroll,
                follow,
                queue_scroll,
                anim_tick,
                now_ms,
                flash_status_text(mode_flash, anim_tick),
                skill_menu,
                task_picker,
                command_menu,
                model_menu,
                cache_salt_menu,
                keymap_menu,
                hits,
                viewport,
                shift_held,
                copy_mode,
                pending_images,
                input_disabled,
                plan_mode,
                edit_title,
                tail_ms,
                task_ms,
                is_top_level,
                ap_enabled,
                display_mode,
            )
        },
        |terminal| end_synchronized_update(terminal.backend_mut()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FlushCountingWriter {
        bytes: Vec<u8>,
        flushes: usize,
    }

    impl Write for FlushCountingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn synchronized_frame_wraps_output_in_mode_2026() {
        let mut output = FlushCountingWriter::default();

        synchronized_frame(
            &mut output,
            begin_synchronized_update,
            |writer| {
                assert_eq!(writer.flushes, 0, "begin must not flush separately");
                writer.write_all(b"frame")?;
                writer.flush()?; // Existing `Terminal::draw` flush.
                Ok(())
            },
            |writer| {
                assert_eq!(writer.flushes, 1, "draw owns the first flush");
                end_synchronized_update(writer)
            },
        )
        .unwrap();

        assert_eq!(output.bytes, b"\x1b[?2026hframe\x1b[?2026l");
        assert_eq!(output.flushes, 2, "end adds exactly one flush per frame");
    }

    #[test]
    fn synchronized_frame_always_ends_after_render_error() {
        let mut output = Vec::new();

        let error = synchronized_frame(
            &mut output,
            begin_synchronized_update,
            |writer| -> anyhow::Result<()> {
                writer.write_all(b"partial")?;
                anyhow::bail!("render failed")
            },
            end_synchronized_update,
        )
        .unwrap_err();

        assert_eq!(output, b"\x1b[?2026hpartial\x1b[?2026l");
        assert_eq!(error.to_string(), "render failed");
    }

    #[test]
    fn synchronized_frame_reports_end_failure() {
        let mut state = (Vec::new(), false);

        let error = synchronized_frame(
            &mut state,
            |(steps, _)| {
                steps.push("begin");
                Ok(())
            },
            |(steps, _)| {
                steps.push("render");
                Ok(())
            },
            |(steps, end_called)| {
                steps.push("end");
                *end_called = true;
                Err(std::io::Error::other("end failed"))
            },
        )
        .unwrap_err();

        assert_eq!(state.0, vec!["begin", "render", "end"]);
        assert!(state.1);
        assert_eq!(error.to_string(), "end failed");
    }
}
