//! Frame rendering extracted from `app_loop.rs` to keep that file under the
//! 800-line cap. `render_frame` resolves the plan-edit/composer display state
//! and delegates to `crate::render::render`.

use std::time::{Duration, Instant};

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

/// Transient copy-status text if still within 2s of firing.
fn copy_status_text(copy_status: &Option<(String, Instant)>) -> Option<&str> {
    copy_status
        .as_ref()
        .and_then(|(m, t)| (t.elapsed() < Duration::from_secs(2)).then_some(m.as_str()))
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
    title: &str,
    agent: &str,
    running: bool,
    show_help: bool,
    ctx: u64,
    sys: u64,
    context_limit: u64,
    model: &str,
    status: &str,
    steer_items: &[(i64, String)],
    queue_items: &[(i64, String)],
    scroll: &mut u32,
    follow: bool,
    anim_tick: u32,
    mode_flash: &Option<(String, u32)>,
    skill_menu: Option<&crate::menu::SkillMenu>,
    task_picker: Option<&crate::task::TaskPicker>,
    command_menu: Option<&crate::command::CommandMenu>,
    model_menu: Option<&crate::model_menu::ModelMenu>,
    cache_salt_menu: Option<&crate::cache_salt_menu::CacheSaltMenu>,
    hits: &mut crate::render::MouseHits,
    viewport: &mut Option<crate::render_viewport::ViewportCache>,
    selection: Option<crate::selection::SelRange>,
    copy_status: &Option<(String, Instant)>,
    pending_images: &[(String, String)],
    input_disabled: bool,
    run_ms: u64,
) -> anyhow::Result<()> {
    let plan_label = plan_edit.as_ref().map(|pe| pe.mode_label());
    let (render_input, render_cursor) = match plan_edit {
        Some(pe) => (pe.text(), pe.cursor()),
        None => (input, cursor_idx),
    };
    let plan_mode: Option<&str> = plan_label.as_deref();
    crate::render::render(
        terminal,
        chat,
        render_input,
        render_cursor,
        title,
        agent,
        running,
        show_help,
        ctx,
        sys,
        context_limit,
        model,
        status,
        steer_items,
        queue_items,
        scroll,
        follow,
        anim_tick,
        flash_status_text(mode_flash, anim_tick),
        skill_menu,
        task_picker,
        command_menu,
        model_menu,
        cache_salt_menu,
        hits,
        viewport,
        selection,
        copy_status_text(copy_status),
        pending_images,
        input_disabled,
        plan_mode,
        run_ms,
    )
}
