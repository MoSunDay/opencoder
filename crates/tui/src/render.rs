//! All TUI rendering functions — body, composer, status bar, popups, cursor.

use std::io::Stdout;

use anyhow::Result;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use ratatui::Terminal;

use crate::cache_salt_menu::CacheSaltMenu;
use crate::chat::ChatView;
use crate::command::CommandMenu;
use crate::composer;
use crate::fmt as fmtmod;
use crate::menu::SkillMenu;
use crate::model_menu::ModelMenu;
use crate::queue_panel::{btn_x_offsets, steer_btn_x_offsets, QueueBtn, QueueBtnAction};
use crate::task::TaskPicker;

pub(crate) type Term = Terminal<CrosstermBackend<Stdout>>;

/// Context baseline subtracted from used/window so small sessions read ~0%.
const CONTEXT_BASELINE: u64 = 4_000;

/// Braille spinner frames shown while a task is running.
const SPINNER: [&str; 10] = [
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}", "\u{2827}",
    "\u{2807}", "\u{280f}",
];

/// Mouse hit-targets exported by `render` for the event loop to test clicks
/// and wheel scrolls against. Recomputed every frame.
#[derive(Default)]
pub(crate) struct MouseHits {
    pub jump_btn: Option<Rect>,
    pub top_btn: Option<Rect>,
    pub body: Option<Rect>,
    pub queue_btns: Vec<QueueBtn>,
    /// Clickable Thinking-block header rows; clicking toggles collapse.
    /// One entry per Thinking block currently visible in the body viewport.
    pub thinking_btns: Vec<ThinkingBtn>,
    /// Clickable Subagent-block header rows; clicking toggles collapse.
    pub subagent_btns: Vec<SubagentBtn>,
}

/// A clickable Thinking-block header. `block_idx` indexes `ChatView::blocks`;
/// `rect` is the on-screen row of the header line.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ThinkingBtn {
    pub block_idx: usize,
    pub rect: Rect,
}

/// A clickable Subagent-block header.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct SubagentBtn {
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
    title: &str,
    agent: &str,
    running: bool,
    show_help: bool,
    context_used: u64,
    sys_tokens: u64,
    context_limit: u64,
    model: &str,
    status: &str,
    steer_items: &[(i64, String)],
    queue_items: &[(i64, String)],
    scroll: &mut u16,
    follow: bool,
    anim_tick: u32,
    mode_flash: Option<&str>,
    skill_menu: Option<&SkillMenu>,
    task_picker: Option<&TaskPicker>,
    command_menu: Option<&CommandMenu>,
    model_menu: Option<&ModelMenu>,
    cache_salt_menu: Option<&CacheSaltMenu>,
    hits: &mut MouseHits,
    selection: Option<crate::selection::SelRange>,
    copy_status: Option<&str>,
) -> Result<()> {
    terminal.draw(|f| {
        let area = f.area();
        let prompt_w = 2u16;
        let inner_w = area.width.saturating_sub(2);
        let input_rows = composer::display_rows(input, inner_w, prompt_w).max(2);
        let composer_h = (input_rows + 2).min(area.height / 3);
        let composer_inner_h = composer_h.saturating_sub(2).max(1);
        let (cur_row, _cur_col) = composer::cursor_row_col(input, cursor_idx, inner_w, prompt_w);
        let max_scroll = input_rows.saturating_sub(composer_inner_h);
        let composer_scroll = (cur_row as u16)
            .saturating_sub(composer_inner_h.saturating_sub(1))
            .min(max_scroll);
        let pending = steer_items.len() + queue_items.len();
        let queue_h = if pending > 0 {
            pending.min(3) as u16
        } else {
            0
        };
        let skill_h = if skill_menu.is_some() { 8 } else { 0 };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(queue_h),
                Constraint::Length(skill_h),
                Constraint::Length(composer_h),
                Constraint::Length(1),
            ])
            .split(area);

        let mut ci = 0;
        hits.queue_btns.clear();
        hits.thinking_btns.clear();
        hits.subagent_btns.clear();
        render_body(
            f,
            chunks[ci],
            chat,
            title,
            scroll,
            follow,
            anim_tick,
            &mut hits.body,
            &mut hits.jump_btn,
            &mut hits.top_btn,
            &mut hits.thinking_btns,
            &mut hits.subagent_btns,
            selection,
        );
        ci += 1;
        if queue_h > 0 {
            render_queue_panel(
                f,
                chunks[ci],
                steer_items,
                queue_items,
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
            composer_scroll,
            inner_w,
            prompt_w,
        );
        let composer_area = chunks[ci];
        ci += 1;
        render_status(
            f,
            chunks[ci],
            running,
            status,
            model,
            agent,
            anim_tick,
            context_used + sys_tokens,
            context_limit,
        );

        if show_help {
            render_help_popup(f, area);
        }
        if let Some(tp) = task_picker {
            crate::task::render_task_picker(f, area, tp);
        }
        if let Some(cm) = command_menu {
            crate::command::render_command_popup(f, area, composer_area.y, cm);
        }
        if let Some(mm) = model_menu {
            crate::model_menu::render_model_popup(f, area, composer_area.y, mm);
        }
        if let Some(cs) = cache_salt_menu {
            crate::cache_salt_menu::render_cache_salt_popup(f, area, cs);
        }
        if let Some(text) = mode_flash {
            let is_plan = text.contains("plan");
            render_status_chip(f, composer_area, text, mode_flash_bg(is_plan));
        }
        if let Some(text) = copy_status {
            render_status_chip(f, composer_area, text, Color::Green);
        }
        place_cursor(
            f,
            composer_area,
            input,
            cursor_idx,
            inner_w,
            prompt_w,
            composer_scroll,
        );
    })?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn render_body(
    f: &mut Frame,
    area: Rect,
    chat: &ChatView,
    title: &str,
    scroll: &mut u16,
    follow: bool,
    anim_tick: u32,
    body_out: &mut Option<Rect>,
    jump_btn: &mut Option<Rect>,
    top_btn: &mut Option<Rect>,
    thinking_btns: &mut Vec<ThinkingBtn>,
    subagent_btns: &mut Vec<SubagentBtn>,
    selection: Option<crate::selection::SelRange>,
) {
    *body_out = Some(area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", title));
    let inner = block.inner(area);
    let visible_h = inner.height as usize;
    let text_w = inner.width.saturating_sub(1);
    let lines = chat.flatten_with(anim_tick);
    let para = Paragraph::new(lines.clone()).wrap(Wrap { trim: false });
    let total_rows = para.line_count(text_w);
    let max_rows = total_rows.saturating_sub(visible_h);
    if follow {
        *scroll = max_rows as u16;
    }
    *scroll = (*scroll as usize).min(max_rows) as u16;
    let scroll_y = *scroll;

    // Record click hit-rects for Thinking-block header lines that fall inside
    // the viewport. We only need the wrapped-row offset of each header line;
    // compute it lazily and skip headers that are clearly off-screen. Since a
    // wrapped line occupies >= 1 screen row, logical line index is a lower
    // bound on screen row, so a header whose line index is beyond the viewport
    // is guaranteed off-screen below and is skipped without any wrapping math.
    record_thinking_hits(
        chat,
        &lines,
        text_w,
        scroll_y as usize,
        visible_h,
        inner.x,
        inner.y,
        thinking_btns,
    );
    record_subagent_hits(
        chat,
        &lines,
        text_w,
        scroll_y as usize,
        visible_h,
        inner.x,
        inner.y,
        subagent_btns,
    );

    f.render_widget(block, area);
    let text_area = Rect {
        height: visible_h as u16,
        width: text_w,
        ..inner
    };
    f.render_widget(para.scroll((scroll_y, 0)), text_area);

    if total_rows > visible_h {
        let scroll_area = Rect {
            height: visible_h as u16,
            ..inner
        };
        draw_scrollbar(f, scroll_area, total_rows, visible_h, scroll_y as usize);
    }

    // Selection highlight — drawn last so it sits on top of the text.
    crate::selection::render_overlay(f, text_area, scroll_y, selection);

    // Follow indicator on the body's bottom-border row, right-aligned.
    let (label, style) = if follow {
        (
            " \u{8ddf}\u{968f}\u{4e2d}\u{2026} ",
            Style::default().fg(Color::Cyan),
        )
    } else {
        (
            "    \u{2b07}    ",
            Style::default()
                .fg(Color::Yellow)
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
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD);
        let top_w: u16 = top_label
            .chars()
            .map(composer::char_width)
            .sum::<usize>() as u16;
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

/// Manual scrollbar with correct thumb positioning even when content barely
/// overflows the viewport. ratatui's `ScrollbarState` inflates the denominator
/// by `viewport − 1`, which parks the thumb mid-track at the bottom when
/// content ≈ viewport. This uses the simple ratio `scroll / max_scroll`.
fn draw_scrollbar(
    f: &mut Frame,
    inner: Rect,
    total_rows: usize,
    visible_h: usize,
    scroll_y: usize,
) {
    let max_scroll = total_rows.saturating_sub(visible_h);
    let track_h = inner.height as u64;
    let thumb_h = (visible_h as u64 * track_h / total_rows as u64).max(1) as u16;
    let max_off = inner.height.saturating_sub(thumb_h);
    let thumb_off = if max_scroll == 0 {
        0u16
    } else {
        ((scroll_y as u64 * max_off as u64) / max_scroll as u64) as u16
    };
    let sb_x = inner.right().saturating_sub(1);
    let buf = f.buffer_mut();
    for y in 0..inner.height {
        let cell = &mut buf[(sb_x, inner.y + y)];
        if y >= thumb_off && y < thumb_off + thumb_h {
            cell.set_char('\u{2592}');
            cell.set_style(Style::default().fg(Color::Gray));
        } else {
            cell.set_char('\u{2502}');
            cell.set_style(Style::default().fg(Color::DarkGray));
        }
    }
}

/// Number of screen rows `line` occupies when word-wrapped at width `w`,
/// matching ratatui's `Paragraph` wrapping exactly. An empty line is 1 row.
fn wrapped_rows(line: &Line<'_>, w: u16) -> usize {
    Paragraph::new(line.clone())
        .wrap(Wrap { trim: false })
        .line_count(w)
}

/// Populate `out` with one `ThinkingBtn` per Thinking-block header line that is
/// currently visible inside the body viewport. Walks flattened lines in order,
/// accumulating wrapped screen rows; stops as soon as headers pass below the
/// viewport. When there are no Thinking blocks the cost is one empty
/// `thinking_headers()` call and nothing more.
#[allow(clippy::too_many_arguments)]
fn record_thinking_hits(
    chat: &ChatView,
    lines: &[Line<'_>],
    text_w: u16,
    scroll_y: usize,
    visible_h: usize,
    x: u16,
    y0: u16,
    out: &mut Vec<ThinkingBtn>,
) {
    let headers = chat.thinking_headers();
    if headers.is_empty() || visible_h == 0 || text_w == 0 {
        return;
    }
    let viewport_bottom = scroll_y + visible_h;
    let mut row: usize = 0; // screen row of the next line to consume
    let mut li: usize = 0; // current logical line index
    for h in headers {
        let target = h.header_line_idx;
        // Advance to the header's line, accumulating wrapped rows of the lines
        // that precede it.
        while li < target && li < lines.len() {
            row += wrapped_rows(&lines[li], text_w);
            li += 1;
        }
        if li >= lines.len() {
            break;
        }
        let header_row = row;
        if header_row >= viewport_bottom {
            // This and all later headers are below the viewport.
            break;
        }
        if header_row >= scroll_y {
            let screen_y = y0.saturating_add((header_row - scroll_y) as u16);
            out.push(ThinkingBtn {
                block_idx: h.block_idx,
                rect: Rect::new(x, screen_y, text_w, 1),
            });
        }
        // Consume the header line and advance to the next header.
        row += wrapped_rows(&lines[li], text_w);
        li += 1;
    }
}

/// Populate `out` with one `SubagentBtn` per Subagent-block header line that is
/// currently visible inside the body viewport. Mirrors `record_thinking_hits`.
#[allow(clippy::too_many_arguments)]
fn record_subagent_hits(
    chat: &ChatView,
    lines: &[Line<'_>],
    text_w: u16,
    scroll_y: usize,
    visible_h: usize,
    x: u16,
    y0: u16,
    out: &mut Vec<SubagentBtn>,
) {
    let headers = chat.subagent_headers();
    if headers.is_empty() || visible_h == 0 || text_w == 0 {
        return;
    }
    let viewport_bottom = scroll_y + visible_h;
    let mut row: usize = 0;
    let mut li: usize = 0;
    for h in headers {
        let target = h.header_line_idx;
        while li < target && li < lines.len() {
            row += wrapped_rows(&lines[li], text_w);
            li += 1;
        }
        if li >= lines.len() {
            break;
        }
        let header_row = row;
        if header_row >= viewport_bottom {
            break;
        }
        if header_row >= scroll_y {
            let screen_y = y0.saturating_add((header_row - scroll_y) as u16);
            out.push(SubagentBtn {
                block_idx: h.block_idx,
                rect: Rect::new(x, screen_y, text_w, 1),
            });
        }
        row += wrapped_rows(&lines[li], text_w);
        li += 1;
    }
}

fn render_composer(
    f: &mut Frame,
    area: Rect,
    input: &str,
    scroll: u16,
    inner_w: u16,
    prompt_w: u16,
) {
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(block, area);
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
            spans.push(Span::styled(
                "\u{276f} ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        let text: String = chars[vr.start..vr.end].iter().collect();
        spans.push(Span::raw(text));
        lines.push(Line::from(spans));
    }
    f.render_widget(
        Paragraph::new(Text::from(lines)).scroll((scroll, 0)),
        inner,
    );
}

#[allow(clippy::too_many_arguments)]
/// Foreground color of the `[agent]` status chip (issue #6): Yellow in
/// read-only plan mode (caution), Cyan for act. Was uniformly Magenta.
/// Shared by the status bar and the `/task` picker so the two stay
/// visually consistent.
pub(crate) fn agent_chip_fg(agent: &str) -> Color {
    if agent == "plan" {
        Color::Yellow
    } else {
        Color::Cyan
    }
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

/// Background color of the plan/act mode-flash chip (issue #6): Yellow for
/// plan, Cyan for act. Extracted pure so the theme mapping is testable.
fn mode_flash_bg(is_plan: bool) -> Color {
    if is_plan {
        Color::Yellow
    } else {
        Color::Cyan
    }
}

#[allow(clippy::too_many_arguments)]
fn render_status(
    f: &mut Frame,
    area: Rect,
    running: bool,
    status: &str,
    model: &str,
    agent: &str,
    anim_tick: u32,
    used: u64,
    limit: u64,
) {
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(model.to_string(), Style::default().fg(Color::White)),
        Span::raw(" | "),
        // Agent chip: plan = Yellow (read-only caution), otherwise Cyan (issue
        // #6 — was uniformly Magenta, clashing with the Cyan theme).
        Span::styled(
            format!("[{agent}]"),
            Style::default().fg(agent_chip_fg(agent)),
        ),
    ];

    // Context-window usage indicator, placed right after the agent chip.
    let pct = fmtmod::context_percent(used, limit, CONTEXT_BASELINE);
    let ctx_color = if pct >= 85 {
        Color::Red
    } else if pct >= 60 {
        Color::Yellow
    } else {
        Color::Green
    };
    spans.push(Span::styled(
        format!(
            " ctx {}% ({}/{})",
            pct,
            fmtmod::format_tokens_compact(used),
            fmtmod::format_tokens_compact(limit)
        ),
        Style::default().fg(ctx_color),
    ));
    spans.push(Span::raw("  "));

    if running {
        let spin = SPINNER[(anim_tick as usize) % SPINNER.len()];
        spans.push(Span::styled(
            format!("{spin} {status}"),
            Style::default().fg(Color::Yellow),
        ));
    } else if !status.is_empty() {
        spans.push(Span::styled(
            format!("| {status}"),
            Style::default().fg(Color::DarkGray),
        ));
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_queue_panel(
    f: &mut Frame,
    area: Rect,
    steer_items: &[(i64, String)],
    queue_items: &[(i64, String)],
    btns: &mut Vec<QueueBtn>,
) {
    struct E<'a> {
        prefix: &'a str,
        text: &'a str,
        color: Color,
        seq: Option<i64>,
        is_steer: bool,
    }
    let mut entries: Vec<E> = Vec::new();
    for (seq, s) in steer_items {
        entries.push(E {
            prefix: "\u{21b3} steer",
            text: s.as_str(),
            color: Color::Blue,
            seq: Some(*seq),
            is_steer: true,
        });
    }
    for (seq, q) in queue_items {
        entries.push(E {
            prefix: "[queued]",
            text: q.as_str(),
            color: Color::Yellow,
            seq: Some(*seq),
            is_steer: false,
        });
    }
    let total = entries.len();
    if total == 0 || area.height == 0 {
        return;
    }

    let max_lines = (area.height as usize).min(3);
    let avail_w = area.width as usize;
    let overflow = total > max_lines;
    let item_capacity = if overflow {
        max_lines.saturating_sub(1)
    } else {
        max_lines
    };
    let start = total.saturating_sub(item_capacity);
    let visible = &entries[start..];

    let mut lines: Vec<Line> = Vec::new();
    if overflow {
        lines.push(Line::from(Span::styled(
            format!(" \u{2191}{} more ", start),
            Style::default().fg(Color::DarkGray),
        )));
    }
    // Clickable rows reserve a trailing control strip. Queue rows use a
    // 6-column strip (" \u{25b2} \u{25bc} \u{2715}": up/down/delete); steer
    // rows use a 4-column strip (" \u{2715} >": delete/submit). Very narrow
    // terminals render without controls. Each control glyph gets a 1-cell
    // hit rect.
    for e in visible {
        let btn_w = if e.is_steer { 4usize } else { 6usize };
        let clickable = e.seq.is_some() && avail_w > btn_w + 4;
        let cap = if clickable {
            avail_w.saturating_sub(btn_w)
        } else {
            avail_w
        };
        let head = format!(" {}: {}", e.prefix, e.text);
        let head_display = composer::truncate_to_width(&head, cap);
        let head_len = composer::str_width(&head_display);
        let mut spans: Vec<Span> = vec![Span::styled(head_display, Style::default().fg(e.color))];
        if clickable {
            let seq = e.seq.unwrap();
            let y = area.y + lines.len() as u16;
            // Right-align the control strip: pad the head out to `cap` so the
            // glyphs land at the right edge and stay aligned with the hit rects.
            let pad = cap.saturating_sub(head_len);
            if pad > 0 {
                spans.push(Span::raw(" ".repeat(pad)));
            }
            if e.is_steer {
                // Steer row: " ✕ >" — delete + submit-now.
                spans.push(Span::styled(
                    " \u{2715} >".to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
                let [del_x, sub_x] = steer_btn_x_offsets(area.width);
                btns.push(QueueBtn {
                    seq,
                    action: QueueBtnAction::Delete,
                    rect: Rect::new(area.x + del_x, y, 1, 1),
                });
                btns.push(QueueBtn {
                    seq,
                    action: QueueBtnAction::Submit,
                    rect: Rect::new(area.x + sub_x, y, 1, 1),
                });
            } else {
                // Queue row: " ▲ ▼ ✕" — up/down/delete.
                spans.push(Span::styled(
                    " \u{25b2} \u{25bc} \u{2715}".to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
                let [up_x, down_x, del_x] = btn_x_offsets(area.width);
                btns.push(QueueBtn {
                    seq,
                    action: QueueBtnAction::Up,
                    rect: Rect::new(area.x + up_x, y, 1, 1),
                });
                btns.push(QueueBtn {
                    seq,
                    action: QueueBtnAction::Down,
                    rect: Rect::new(area.x + down_x, y, 1, 1),
                });
                btns.push(QueueBtn {
                    seq,
                    action: QueueBtnAction::Delete,
                    rect: Rect::new(area.x + del_x, y, 1, 1),
                });
            }
        }
        lines.push(Line::from(spans));
    }

    f.render_widget(Paragraph::new(lines), area);
}

fn render_help_popup(f: &mut Frame, area: Rect) {
    let h = 20u16.min(area.height.saturating_sub(2));
    let w = 60u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Help (Ctrl+H, Esc to close) ");
    f.render_widget(Paragraph::new(crate::keybind::HELP).block(block), popup);
}

fn place_cursor(
    f: &mut Frame,
    composer_area: Rect,
    input: &str,
    cursor_idx: usize,
    inner_w: u16,
    prompt_w: u16,
    scroll: u16,
) {
    let border = 1u16;
    let (row, col) = composer::cursor_row_col(input, cursor_idx, inner_w, prompt_w);
    let x = if row == 0 {
        composer_area.x + border + prompt_w + col as u16
    } else {
        composer_area.x + border + col as u16
    };
    let y = composer_area.y + border + (row as u16).saturating_sub(scroll);
    f.set_cursor_position((x, y));
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
