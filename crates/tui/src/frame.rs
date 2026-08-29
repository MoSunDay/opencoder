//! Frame rendering extracted from `app_loop.rs` to keep that file under the
//! 800-line cap. `render_frame` resolves the plan-edit/composer display state
//! and delegates to `crate::render::render`.

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

/// Whether the mode-flash chip renders in the warning hue: the read-only
/// sandbox family, the plan-text editor, and the clear-context countdown
/// guard (a destructive operation about to fold the transcript).
pub(crate) fn is_warn_flash(text: &str) -> bool {
    text.starts_with("\u{2192} sandbox mode")
        || text.starts_with("\u{2192} plan mode")
        || text.starts_with("\u{2192} clear")
}

/// Transient mode-flash status text if still within its visibility window.
fn flash_status_text(mode_flash: &Option<(String, u32)>, anim_tick: u32) -> Option<&str> {
    mode_flash
        .as_ref()
        .and_then(|(t, s)| flash_visible(*s, anim_tick, MODE_FLASH_TICKS).then_some(t.as_str()))
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
    file_menu: Option<&crate::file_menu::FileMenu>,
    model_menu: Option<&crate::model_menu::ModelMenu>,
    mcp_menu: Option<&crate::mcp_menu::McpMenu>,
    envs_menu: Option<&crate::envs_menu::EnvsMenu>,
    cli_menu: Option<&crate::cli_menu::CliMenu>,
    skill_toggle_menu: Option<&crate::skill_menu::SkillMenu>,
    ap_menu: Option<&crate::ap_menu::ApMenu>,
    cache_salt_menu: Option<&crate::cache_salt_menu::CacheSaltMenu>,
    keymap_menu: Option<&crate::keymap_menu::KeymapMenu>,
    question_menu: Option<&crate::question_menu::QuestionMenu>,
    hits: &mut crate::render::MouseHits,
    viewport: &mut Option<crate::render_viewport::ViewportCache>,
    shift_held: bool,
    copy_mode: bool,
    pending_images: &[(String, String)],
    input_disabled: bool,
    tail_ms: u64,
    task_ms: u64,
    is_top_level: bool,
    ap_mode: opencoder_core::ApMode,
    display_mode: &str,
    notepad: Option<&crate::notepad::NotepadView>,
) -> anyhow::Result<()> {
    let plan_label = plan_edit.as_ref().map(|pe| pe.mode_label());
    let (render_input, render_cursor) = match plan_edit {
        Some(pe) => (pe.text(), pe.cursor()),
        None => (input, cursor_idx),
    };
    let plan_mode: Option<&str> = plan_label.as_deref();
    let edit_title: Option<&str> = plan_edit.as_ref().map(|pe| pe.title());
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
        file_menu,
        model_menu,
        mcp_menu,
        envs_menu,
        cli_menu,
        skill_toggle_menu,
        ap_menu,
        cache_salt_menu,
        keymap_menu,
        question_menu,
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
        ap_mode,
        display_mode,
        notepad,
    )
}

#[cfg(test)]
#[path = "frame/tests.rs"]
mod tests;
