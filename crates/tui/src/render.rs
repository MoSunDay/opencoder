//! All TUI rendering functions — body, composer, status bar, popups, cursor.

use std::io::Stdout;

use anyhow::Result;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use ratatui::Terminal;

use crate::cache_salt_menu::CacheSaltMenu;
use crate::chat::ChatView;
use crate::command::CommandMenu;
use crate::composer;
use crate::fmt as fmtmod;
use crate::keymap_menu::KeymapMenu;
use crate::menu::SkillMenu;
use crate::model_menu::ModelMenu;
use crate::queue_panel::QueueBtn;
use crate::render_viewport::ViewportCache;
use crate::task::TaskPicker;
use crate::theme;

pub(crate) type Term = Terminal<CrosstermBackend<Stdout>>;

#[path = "render_status.rs"]
mod status_bar;
use status_bar::render_status;

/// Mouse hit-targets exported by `render` for the event loop to test clicks
/// and wheel scrolls against. Recomputed every frame.
#[derive(Default)]
pub(crate) struct MouseHits {
    pub jump_btn: Option<Rect>,
    pub top_btn: Option<Rect>,
    pub body: Option<Rect>,
    /// Queue/steer panel area (Some while the panel is visible), used by the
    /// scroll-wheel handler to scroll the panel instead of the body.
    pub queue_panel: Option<Rect>,
    /// Cached total pending entries (steer + queue) from the last render.
    /// Mirrors `total_rows` for the body: lets the wheel handler clamp the
    /// queue scroll without re-deriving the panel contents.
    pub queue_total: usize,
    pub queue_btns: Vec<QueueBtn>,
    /// Clickable Thinking-block header rows; clicking toggles collapse.
    /// One entry per Thinking block currently visible in the body viewport.
    pub thinking_btns: Vec<ThinkingBtn>,
    /// Clickable Subagent-block header rows; clicking toggles collapse.
    pub subagent_btns: Vec<SubagentBtn>,
    /// Clickable Tool-block header rows; clicking toggles collapse.
    /// One entry per Tool block currently visible in the body viewport.
    pub tool_btns: Vec<ToolBtn>,
    /// Clickable Compaction-block header rows; clicking toggles collapse.
    pub compaction_btns: Vec<CompactionBtn>,
    pub keymap_btns: Vec<Rect>,
    /// Cached total content rows from the last render_body call. Used by
    /// the scroll-wheel handler to clamp scroll without re-flattening.
    pub total_rows: usize,
}

/// A clickable Thinking-block header. `block_idx` indexes `ChatView::blocks`;
/// `rect` is the on-screen row of the header line.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ThinkingBtn {
    pub block_idx: usize,
    pub rect: Rect,
}

/// A clickable Subagent-block header.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SubagentBtn {
    pub block_idx: usize,
    pub rect: Rect,
}

/// A clickable Tool-block header.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ToolBtn {
    pub block_idx: usize,
    pub rect: Rect,
}

pub(crate) fn in_rect(r: Rect, col: u16, row: u16) -> bool {
    col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render<B: Backend>(
    terminal: &mut Terminal<B>,
    chat: &ChatView,
    input: &str,
    cursor_idx: usize,
    title: &Line<'static>,
    running: bool,
    context_used: u64,
    sys_tokens: u64,
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
    mode_flash: Option<&str>,
    skill_menu: Option<&SkillMenu>,
    task_picker: Option<&TaskPicker>,
    command_menu: Option<&CommandMenu>,
    model_menu: Option<&ModelMenu>,
    mcp_menu: Option<&crate::mcp_menu::McpMenu>,
    envs_menu: Option<&crate::envs_menu::EnvsMenu>,
    cli_menu: Option<&crate::cli_menu::CliMenu>,
    skill_toggle_menu: Option<&crate::skill_menu::SkillMenu>,
    ap_menu: Option<&crate::ap_menu::ApMenu>,
    cache_salt_menu: Option<&CacheSaltMenu>,
    keymap_menu: Option<&KeymapMenu>,
    question_menu: Option<&crate::question_menu::QuestionMenu>,
    hits: &mut MouseHits,
    viewport: &mut Option<ViewportCache>,
    shift_held: bool,
    copy_mode: bool,
    pending_images: &[(String, String)],
    input_disabled: bool,
    plan_mode: Option<&str>,
    edit_title: Option<&str>,
    tail_ms: u64,
    task_ms: u64,
    is_top_level: bool,
    ap_mode: opencoder_core::ApMode,
    display_mode: &str,
    notepad: Option<&crate::notepad::NotepadView>,
) -> Result<()> {
    terminal.draw(|f| {
        let area = f.area();
        // When notepad is open it takes the whole terminal (fullscreen file
        // viewer/editor): render tree+editor over the entire frame, clear
        // every chat hit-target so no stale rect survives from the previous
        // chat frame, and skip all chat rendering below (body/composer/status).
        if let Some(np) = notepad {
            hits.body = None;
            hits.jump_btn = None;
            hits.top_btn = None;
            hits.queue_panel = None;
            hits.queue_total = 0;
            hits.total_rows = 0;
            hits.queue_btns.clear();
            hits.thinking_btns.clear();
            hits.subagent_btns.clear();
            hits.tool_btns.clear();
            hits.compaction_btns.clear();
            hits.keymap_btns.clear();
            // Copy mode: undecorated fullscreen editor text — no tree
            // panel, no border, no line-number gutter — so native
            // selection spans pure file text (see copy_mode::render_clean
            // for the body's counterpart).
            if copy_mode {
                crate::copy_mode::render_notepad_clean(f, area, np);
            } else {
                crate::notepad::render_top(f, area, np);
            }
            return;
        }
        let draw_area = area;
        // Ratatui resets the next diff buffer after every completed draw, and
        // the persistent widgets below cover their full rectangles. Do not
        // clear the entire frame here: that adds an O(viewport cells) pass to
        // the hot path without fixing terminal-side partial-frame exposure.
        // The normal ratatui diff overwrites every changed or vacated cell.
        // Steady-state frames never clear; lifecycle transitions and the
        // explicit force-redraw command own isolated physical clears.
        let prompt_w = 2u16;
        let inner_w = draw_area.width.saturating_sub(2);
        let input_rows = composer::display_rows(input, inner_w, prompt_w).max(2);
        let plan_active = plan_mode.is_some();
        // The attachment badge consumes one inner line; must mirror the
        // plan-mode filter applied at the render_composer call site below.
        let badge_h: u16 = if !plan_active && !pending_images.is_empty() {
            1
        } else {
            0
        };
        let pending = steer_items.len() + queue_items.len();
        let queue_h = if plan_active {
            0
        } else if pending > 0 {
            pending.min(3) as u16
        } else {
            0
        };
        let skill_h = if plan_active {
            0
        } else if skill_menu.is_some() {
            8
        } else {
            0
        };
        let composer_h = if plan_active {
            draw_area.height.saturating_sub(queue_h + skill_h + 1)
        } else {
            (input_rows + 2 + badge_h).min(draw_area.height / 3)
        };
        let composer_inner_h = composer_h.saturating_sub(2).max(1);
        let (cur_row, _cur_col) = composer::cursor_row_col(input, cursor_idx, inner_w, prompt_w);
        // The badge steals one visible inner line, so subtract it from the
        // usable text height for both scroll bounds.
        let text_h = composer_inner_h.saturating_sub(badge_h).max(1);
        let max_scroll = input_rows.saturating_sub(text_h);
        let composer_scroll = (cur_row as u16)
            .saturating_sub(text_h.saturating_sub(1))
            .min(max_scroll);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                if plan_active {
                    Constraint::Length(0)
                } else {
                    Constraint::Min(3)
                },
                Constraint::Length(queue_h),
                Constraint::Length(skill_h),
                Constraint::Length(composer_h),
                Constraint::Length(1),
            ])
            .split(draw_area);

        let mut ci = 0;
        hits.queue_btns.clear();
        hits.thinking_btns.clear();
        hits.subagent_btns.clear();
        hits.tool_btns.clear();
        hits.compaction_btns.clear();
        hits.keymap_btns.clear();
        if !plan_active {
            render_body(
                f,
                chunks[ci],
                chat,
                title,
                scroll,
                follow,
                anim_tick,
                now_ms,
                &mut hits.body,
                &mut hits.jump_btn,
                &mut hits.top_btn,
                &mut hits.thinking_btns,
                &mut hits.subagent_btns,
                &mut hits.tool_btns,
                &mut hits.compaction_btns,
                viewport,
                is_top_level,
                tail_ms,
                copy_mode,
            );
            // Expose cached total_rows for scroll-wheel clamping.
            hits.total_rows = viewport.as_ref().map_or(0, |v| v.total_rows());
        }
        ci += 1;
        // Clamp a stale queue scroll every frame (entries deleted/consumed
        // since the last interaction shrink the panel) — same pattern as the
        // body `total_rows` clamp in `render_body`.
        hits.queue_panel = None;
        if queue_h > 0 {
            hits.queue_panel = Some(chunks[ci]);
            hits.queue_total = pending;
            let max_scroll = pending.saturating_sub(queue_h as usize);
            *queue_scroll = (*queue_scroll as usize).min(max_scroll) as u32;
            crate::queue_panel::render_queue_panel(
                f,
                chunks[ci],
                steer_items,
                queue_items,
                *queue_scroll,
                &mut hits.queue_btns,
            );
        }
        ci += 1;
        if skill_h > 0 {
            if let Some(menu) = skill_menu {
                crate::menu::render_skill_in_rect(f, chunks[ci], menu);
            }
        }
        ci += 1;
        render_composer(
            f,
            chunks[ci],
            input,
            copy_mode,
            composer_scroll,
            inner_w,
            prompt_w,
            if plan_mode.is_some() {
                &[]
            } else {
                pending_images
            },
            input_disabled,
            plan_mode,
            edit_title,
            title,
        );
        let composer_area = chunks[ci];
        ci += 1;
        render_status(
            f,
            chunks[ci],
            display_mode,
            running,
            status,
            anim_tick,
            context_used + sys_tokens,
            compaction_threshold,
            context_limit,
            task_ms,
        );

        if let Some(tp) = task_picker {
            crate::task::render_task_picker(f, area, tp);
        }
        if let Some(cm) = command_menu {
            crate::command::render_command_popup(f, area, composer_area.y, cm);
        }
        if let Some(mm) = model_menu {
            crate::model_menu::render_model_popup(f, area, composer_area.y, mm);
        }
        if let Some(mcp) = mcp_menu {
            crate::mcp_menu::render_mcp_popup(f, area, composer_area.y, mcp);
        }
        if let Some(envs) = envs_menu {
            crate::envs_menu::render_envs_popup(f, area, composer_area.y, envs);
        }
        if let Some(cli) = cli_menu {
            crate::cli_menu::render_cli_popup(f, area, composer_area.y, cli);
        }
        if let Some(sk) = skill_toggle_menu {
            crate::skill_menu::render_skill_popup(f, area, composer_area.y, sk);
        }
        if let Some(am) = ap_menu {
            crate::ap_menu::render_ap_popup(f, area, composer_area.y, am);
        }
        if let Some(cs) = cache_salt_menu {
            crate::cache_salt_menu::render_cache_salt_popup(f, area, cs);
        }
        if let Some(km) = keymap_menu {
            crate::keymap_menu::render_keymap_popup(f, area, km, &mut hits.keymap_btns);
        }
        if let Some(qm) = question_menu {
            crate::question_menu::render_question_popup(f, area, composer_area.y, qm);
        }
        if let Some(label) = crate::ap_menu::chip_label(ap_mode) {
            render_status_chip(f, composer_area, label, theme::local_color());
        }
        if let Some(text) = mode_flash {
            let is_plan = text.contains("plan");
            render_status_chip(f, composer_area, text, theme::mode_flash_bg(is_plan));
        }
        if shift_held {
            render_status_chip(f, composer_area, "Shift+drag: select", theme::warn_color());
        }
        if copy_mode {
            render_status_chip(
                f,
                composer_area,
                "COPY MODE: Ctrl+G/Esc",
                theme::warn_color(),
            );
        }
        // Popups that own an editable text field (/cli form + content
        // dialog, /mcp form, /model, question) place the terminal cursor
        // themselves; the composer cursor must not override theirs — the
        // last `set_cursor_position` in a frame wins.
        if !input_disabled
            && !copy_mode
            && model_menu.is_none()
            && question_menu.is_none()
            && cli_menu.is_none()
            && mcp_menu.is_none()
            && envs_menu.is_none()
            && skill_toggle_menu.is_none()
        {
            let position = composer::cursor_screen_position(
                composer_area.x,
                composer_area.y,
                input,
                cursor_idx,
                inner_w,
                prompt_w,
                composer_scroll,
                badge_h,
            );
            f.set_cursor_position(position);
        }
    })?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_body(
    f: &mut Frame,
    area: Rect,
    chat: &ChatView,
    title: &Line<'static>,
    scroll: &mut u32,
    follow: bool,
    anim_tick: u32,
    now_ms: i64,
    body_out: &mut Option<Rect>,
    jump_btn: &mut Option<Rect>,
    top_btn: &mut Option<Rect>,
    thinking_btns: &mut Vec<ThinkingBtn>,
    subagent_btns: &mut Vec<SubagentBtn>,
    tool_btns: &mut Vec<ToolBtn>,
    compaction_btns: &mut Vec<CompactionBtn>,
    viewport: &mut Option<ViewportCache>,
    is_top_level: bool,
    tail_ms: u64,
    copy_mode: bool,
) {
    *body_out = Some(area);
    // Copy mode: undecorated full-width view so terminal-native selection
    // spans clean text (no border/scrollbar/timer/indicator rows).
    if copy_mode {
        crate::copy_mode::render_clean(f, area, chat, scroll, follow, anim_tick, now_ms, viewport);
        return;
    }
    let block = theme::rounded_block_line(title);
    let inner = block.inner(area);
    let visible_h = inner.height as usize;
    let text_w = inner.width.saturating_sub(1);

    // B5: Early exit for degenerate terminal sizes — prevents division-by-zero
    // and empty-paragraph panics in a 1x1 terminal.
    if text_w == 0 || visible_h == 0 {
        f.render_widget(block, area);
        return;
    }

    // Empty session: show the in-body tutorial instead of a blank transcript.
    // It vanishes automatically once the first block appears.
    if is_top_level && chat.blocks.is_empty() && !chat.submitted {
        f.render_widget(block, area);
        crate::welcome::render_tutorial_in_body(f, inner);
        return;
    }

    // A1: Build or refresh the viewport cache (rebuilt on first frame,
    // body-refresh invalidation, or width change). Cached lines/offsets
    // make per-frame cost O(visible_h) instead of O(total_content).
    let needs_rebuild = viewport.as_ref().is_none_or(|v| v.width() != text_w);
    if needs_rebuild {
        *viewport = Some(ViewportCache::build(chat, text_w, anim_tick, now_ms));
    }
    let cache = viewport.as_ref().unwrap();
    let total_rows = cache.total_rows();

    // Reserve a row for the [turn cost] timer line when it is visible so the
    // content window never grows large enough to clip it.
    let show_timer = tail_ms > 0;
    let content_h = visible_h.saturating_sub(if show_timer { 1 } else { 0 });

    let max_rows = total_rows.saturating_sub(content_h);
    if follow {
        *scroll = max_rows as u32;
    }
    *scroll = (*scroll as usize).min(max_rows) as u32;
    let scroll_y = *scroll as usize;

    // Record click hit-rects using cached row offsets — O(headers), not O(n).
    hit_records::record_thinking_hits(
        chat,
        cache,
        text_w,
        scroll_y,
        content_h,
        inner.x,
        inner.y,
        thinking_btns,
    );
    hit_records::record_subagent_hits(
        chat,
        cache,
        text_w,
        scroll_y,
        content_h,
        inner.x,
        inner.y,
        subagent_btns,
    );
    hit_records::record_tool_hits(
        chat, cache, text_w, scroll_y, content_h, inner.x, inner.y, tool_btns,
    );
    hit_records::record_compaction_hits(
        chat,
        cache,
        text_w,
        scroll_y,
        content_h,
        inner.x,
        inner.y,
        compaction_btns,
    );

    f.render_widget(block, area);
    let content_area = Rect {
        height: content_h as u16,
        width: text_w,
        ..inner
    };

    // A1: Virtualization — slice only the visible window from cached lines
    // instead of passing the entire transcript to Paragraph. This avoids
    // ratatui internally processing all lines for wrapping/rendering.
    let (start, end, top_skip) = cache.visible_window(scroll_y, content_h);
    let visible_lines: Vec<Line> = cache.lines()[start..end].to_vec();
    let para = Paragraph::new(visible_lines).wrap(Wrap { trim: false });
    f.render_widget(para.scroll((top_skip as u16, 0)), content_area);

    // Per-round [turn cost] timer on a dedicated bottom row.
    if show_timer {
        let timer_area = Rect {
            y: inner.y + content_h as u16,
            height: 1,
            width: text_w,
            x: inner.x,
        };
        let timer = Span::styled(
            format!("[turn cost {}]", fmtmod::format_run_duration(tail_ms)),
            Style::default().fg(theme::warn_color()),
        );
        f.render_widget(Paragraph::new(Line::from(timer)), timer_area);
    }

    if total_rows > visible_h {
        let scroll_area = Rect {
            height: visible_h as u16,
            ..inner
        };
        draw_scrollbar(f, scroll_area, total_rows, content_h, scroll_y);
    }

    // Follow indicator on the body's bottom-border row, right-aligned.
    let (label, style) = if follow {
        (
            " \u{8ddf}\u{968f}\u{4e2d}\u{2026} ",
            Style::default().fg(theme::accent()),
        )
    } else {
        (
            "    \u{2b07}    ",
            Style::default()
                .fg(theme::warn_color())
                .add_modifier(Modifier::BOLD),
        )
    };
    let disp_w: u16 = label.chars().map(composer::char_width).sum::<usize>() as u16;
    let lbl_w = disp_w.min(area.width);
    let lbl_rect = Rect::new(
        area.right().saturating_sub(1).saturating_sub(lbl_w),
        area.bottom().saturating_sub(1),
        lbl_w,
        1,
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(label, style)])),
        lbl_rect,
    );
    *jump_btn = if follow { None } else { Some(lbl_rect) };

    // Top-jump arrow on the body's top-border row, right-aligned. Shown only
    // when scrolled past the top (there is somewhere to scroll up to). Unlike
    // the bottom follow/jump indicator this carries no label — click to jump
    // straight to the very first row.
    if scroll_y > 0 {
        let top_label = "    \u{2b06}    ";
        let top_style = Style::default()
            .fg(theme::warn_color())
            .add_modifier(Modifier::BOLD);
        let top_w: u16 = top_label.chars().map(composer::char_width).sum::<usize>() as u16;
        let top_w = top_w.min(area.width);
        let top_rect = Rect::new(
            area.right().saturating_sub(1).saturating_sub(top_w),
            area.y,
            top_w,
            1,
        );
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(top_label, top_style)])),
            top_rect,
        );
        *top_btn = Some(top_rect);
    }
}

/// Manual scrollbar with correct thumb positioning (ratatui's
/// `ScrollbarState` inflates the denominator, parking the thumb mid-track).
fn draw_scrollbar(
    f: &mut Frame,
    inner: Rect,
    total_rows: usize,
    visible_h: usize,
    scroll_y: usize,
) {
    crate::scrollbar::draw(
        f,
        inner,
        total_rows,
        visible_h,
        scroll_y,
        theme::muted(),
        theme::subtle(),
    );
}

#[allow(clippy::too_many_arguments)]
fn render_composer(
    f: &mut Frame,
    area: Rect,
    input: &str,
    copy_mode: bool,
    scroll: u16,
    inner_w: u16,
    prompt_w: u16,
    pending_images: &[(String, String)],
    disabled: bool,
    plan_mode: Option<&str>,
    edit_title: Option<&str>,
    top_title: &Line<'static>,
) {
    // Copy mode: undecorated input text — no block/border, no prompt
    // glyph, no attachment badge — so terminal-native selection spans
    // exactly the typed text (mirrors the body's clean view).
    if copy_mode {
        crate::copy_mode::render_composer_clean(f, area, input, plan_mode.is_some());
        return;
    }
    if disabled {
        let dim = Style::default()
            .fg(theme::muted())
            .add_modifier(Modifier::DIM);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(dim);
        let inner = block.inner(area);
        f.render_widget(block, area);
        let hint = "subagent ended \u{2014} esc to return";
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("\u{276f} ", dim),
                Span::styled(hint, dim),
            ])),
            inner,
        );
        return;
    }
    let block = if let Some(label) = plan_mode {
        let is_annotation = edit_title == Some("edit annotation");
        let border_fg = if is_annotation {
            theme::ok_color()
        } else {
            theme::warn_color()
        };
        let title_text = edit_title.unwrap_or("edit plan");
        let mut block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_fg))
            .title(format!(" {title_text} "))
            .title_bottom(Line::from(format!(" {label} ")).alignment(Alignment::Left));
        if is_annotation {
            block = block.title(
                theme::title_spans_colored(top_title, theme::ok_color())
                    .alignment(Alignment::Right),
            );
        }
        block
    } else {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme::muted()))
    };
    let inner = block.inner(area);
    f.render_widget(block, area);
    // Attachment indicator: show filenames of pending images above the input.
    // Render the badge on the first inner line and shift the input area down by
    // one row so the text is not overwritten.
    let inner_input = if !pending_images.is_empty() {
        let count = pending_images.len();
        let names: Vec<&str> = pending_images.iter().map(|(_, n)| n.as_str()).collect();
        let label = if count == 1 {
            format!("\u{1f4ce} {}", names[0])
        } else {
            format!("\u{1f4ce} {} \u{00d7}{count}", names[0])
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                label,
                Style::default().fg(theme::warn_color()),
            ))),
            inner,
        );
        // Return area shifted down by 1 line for the input.
        Rect::new(
            inner.x,
            inner.y + 1,
            inner.width,
            inner.height.saturating_sub(1),
        )
    } else {
        inner
    };
    // Pre-split the input into visual rows using the SAME `wrap_rows` model the
    // cursor math derives from, then render each row as an explicit `Line`
    // WITHOUT ratatui's own `.wrap()`. This is the fix for cursor misalignment
    // after soft-wrapping: previously the renderer used ratatui word-wrap while
    // the cursor math used greedy char-wrap, so wrapped points diverged.
    let rows = composer::wrap_rows(input, inner_w, prompt_w);
    let chars: Vec<char> = input.chars().collect();
    let mut lines: Vec<Line> = Vec::new();
    for (ri, vr) in rows.iter().enumerate() {
        let mut spans: Vec<Span> = Vec::new();
        if ri == 0 {
            let prompt_color = if plan_mode.is_some() {
                if edit_title == Some("edit annotation") {
                    theme::ok_color()
                } else {
                    theme::warn_color()
                }
            } else {
                theme::accent()
            };
            spans.push(Span::styled(
                "\u{276f} ",
                Style::default()
                    .fg(prompt_color)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        if ri > 0 {
            spans.push(Span::raw(" ".repeat(prompt_w as usize)));
        }
        let text: String = chars[vr.start..vr.end].iter().collect();
        spans.push(Span::raw(text));
        lines.push(Line::from(spans));
    }
    f.render_widget(
        Paragraph::new(Text::from(lines)).scroll((scroll, 0)),
        inner_input,
    );
}

/// Render a 1-row chip (status bubble) at the top-right of the composer area.
/// Shared by the mode-flash and copy-status overlays so both use identical
/// positioning and layout. `bg` controls the background colour.
fn render_status_chip(f: &mut Frame, composer_area: Rect, text: &str, bg: Color) {
    let pad = 1u16;
    let text_w = composer::str_width(text) as u16;
    let chip_w = text_w.saturating_add(pad.saturating_mul(2));
    let avail = composer_area.width.saturating_sub(2);
    let w = chip_w.min(avail);
    let row = composer_area.y;
    let x = composer_area.x + composer_area.width.saturating_sub(w).saturating_sub(1);
    let chip_rect = Rect {
        x,
        y: row,
        width: w,
        height: 1,
    };
    f.render_widget(Clear, chip_rect);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {text} "),
            Style::default()
                .fg(Color::Black)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        ))),
        chip_rect,
    );
}

#[path = "render_hits.rs"]
mod hit_records;
pub(crate) use hit_records::CompactionBtn;
#[cfg(test)]
#[path = "render_tests/mod.rs"]
mod tests;

#[cfg(test)]
#[path = "render_clear_tests.rs"]
mod clear_tests;
