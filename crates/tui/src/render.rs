//! All TUI rendering functions — body, composer, status bar, popups, cursor.

use std::io::Stdout;
use std::path::Path;

use anyhow::Result;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap,
};
use ratatui::Frame;
use ratatui::Terminal;

use crate::chat::ChatView;
use crate::command::CommandMenu;
use crate::composer;
use crate::fmt as fmtmod;
use crate::menu::SkillMenu;
use crate::model_menu::ModelMenu;
use crate::queue_panel::{btn_x_offsets, QueueBtn, QueueBtnAction};
use crate::task::TaskPicker;

pub(crate) type Term = Terminal<CrosstermBackend<Stdout>>;

/// Context baseline subtracted from used/window so small sessions read ~0%.
const CONTEXT_BASELINE: u64 = 4_000;

/// Braille spinner frames shown while a task is running.
const SPINNER: [&str; 10] = [
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}",
    "\u{2834}", "\u{2826}", "\u{2827}", "\u{2807}", "\u{280f}",
];

/// Mouse hit-targets exported by `render` for the event loop to test clicks
/// and wheel scrolls against. Recomputed every frame.
#[derive(Default)]
pub(crate) struct MouseHits {
    pub jump_btn: Option<Rect>,
    pub body: Option<Rect>,
    pub queue_btns: Vec<QueueBtn>,
}

pub(crate) fn in_rect(r: Rect, col: u16, row: u16) -> bool {
    col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render(
    terminal: &mut Term,
    chat: &ChatView,
    input: &str,
    cursor_idx: usize,
    agent: &str,
    running: bool,
    show_help: bool,
    context_used: u64,
    sys_tokens: u64,
    context_limit: u64,
    model: &str,
    workdir: &Path,
    status: &str,
    steer_items: &[String],
    queue_items: &[(i64, String)],
    scroll: &mut u16,
    follow: bool,
    anim_tick: u32,
    active_skill: Option<&str>,
    skill_menu: Option<&SkillMenu>,
    task_picker: Option<&TaskPicker>,
    command_menu: Option<&CommandMenu>,
    model_menu: Option<&ModelMenu>,
    hits: &mut MouseHits,
) -> Result<()> {
    terminal.draw(|f| {
        let area = f.area();
        let prompt_w = 2u16;
        let composer_inner_w = area.width.saturating_sub(2 + prompt_w);
        let input_rows = composer::display_rows(input, composer_inner_w).max(2);
        let composer_h = (input_rows + 2).min(area.height / 3);
        let pending = steer_items.len() + queue_items.len();
        let queue_h = if pending > 0 { pending.min(3) as u16 } else { 0 };
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
        render_body(f, chunks[ci], chat, scroll, follow, &mut hits.body);
        ci += 1;
        if queue_h > 0 {
            render_queue_panel(f, chunks[ci], steer_items, queue_items, &mut hits.queue_btns);
        }
        ci += 1;
        if skill_h > 0 {
            if let Some(menu) = skill_menu {
                crate::menu::render_skill_in_rect(f, chunks[ci], menu);
            }
        }
        ci += 1;
        render_composer(f, chunks[ci], input, follow, &mut hits.jump_btn);
        let composer_area = chunks[ci];
        ci += 1;
        render_status(
            f, chunks[ci], running, status, steer_items.len() as u32, queue_items.len() as u32,
            model, agent, workdir, context_used + sys_tokens, context_limit,
            anim_tick, active_skill,
        );

        if show_help {
            render_help_popup(f, area);
        }
        if let Some(tp) = task_picker {
            crate::task::render_task_picker(f, area, tp);
        }
        if let Some(cm) = command_menu {
            crate::command::render_command_popup(f, area, cm);
        }
        if let Some(mm) = model_menu {
            crate::model_menu::render_model_popup(f, area, mm);
        }
        place_cursor(f, composer_area, input, cursor_idx);
    })?;
    Ok(())
}

fn render_body(
    f: &mut Frame,
    area: Rect,
    chat: &ChatView,
    scroll: &mut u16,
    follow: bool,
    body_out: &mut Option<Rect>,
) {
    *body_out = Some(area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", chat.agent));
    let inner = block.inner(area);
    let visible_h = inner.height as usize;
    let text_w = inner.width.saturating_sub(1);
    let lines = chat.flatten();
    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    let total_rows = para.line_count(text_w);
    let max_rows = total_rows.saturating_sub(visible_h);
    if follow {
        *scroll = max_rows as u16;
    }
    *scroll = (*scroll as usize).min(max_rows) as u16;
    let scroll_y = *scroll;

    f.render_widget(block, area);
    let text_area = Rect { width: text_w, ..inner };
    f.render_widget(para.scroll((scroll_y, 0)), text_area);

    if total_rows > visible_h {
        let sb_area = Rect::new(inner.right(), inner.y, 1, inner.height);
        let mut sb_state = ScrollbarState::new(total_rows)
            .viewport_content_length(visible_h)
            .position(scroll_y as usize);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            sb_area,
            &mut sb_state,
        );
    }
}

fn render_composer(
    f: &mut Frame,
    area: Rect,
    input: &str,
    follow: bool,
    jump_btn: &mut Option<Rect>,
) {
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let line = Line::from(vec![
        Span::styled(
            "\u{276f} ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw(input.to_string()),
    ]);
    f.render_widget(Paragraph::new(line).wrap(Wrap { trim: false }), inner);

    let (label, style) = if follow {
        ("\u{8ddf}\u{968f}\u{4e2d}\u{2026}", Style::default().fg(Color::Cyan))
    } else {
        ("\u{2193}", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
    };
    let disp_w: u16 = label.chars().map(composer::char_width).sum::<usize>() as u16;
    let lbl_w = disp_w.saturating_add(2).min(area.width);
    let lbl_rect = Rect::new(
        area.right().saturating_sub(1).saturating_sub(lbl_w),
        area.y,
        lbl_w,
        1,
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(label, style)])),
        lbl_rect,
    );
    *jump_btn = if follow { None } else { Some(lbl_rect) };
}

#[allow(clippy::too_many_arguments)]
fn render_status(
    f: &mut Frame,
    area: Rect,
    running: bool,
    status: &str,
    steer_count: u32,
    queue_count: u32,
    model: &str,
    agent: &str,
    workdir: &Path,
    used: u64,
    limit: u64,
    anim_tick: u32,
    active_skill: Option<&str>,
) {
    let pct = fmtmod::context_percent(used, limit, CONTEXT_BASELINE);
    let ctx_color = if pct >= 85 { Color::Red } else if pct >= 60 { Color::Yellow } else { Color::Green };
    let dir_name = workdir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".into());

    let mut spans = vec![
        Span::styled(" opencoder ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("| "),
        Span::styled(model.to_string(), Style::default().fg(Color::White)),
        Span::raw(" | "),
        Span::styled(format!("[{agent}]"), Style::default().fg(Color::Magenta)),
        Span::raw(" | "),
        Span::styled(dir_name, Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled(
            format!("ctx {}% ({}/{})", pct, fmtmod::format_tokens_compact(used), fmtmod::format_tokens_compact(limit)),
            Style::default().fg(ctx_color),
        ),
    ];

    if let Some(name) = active_skill {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("skill:{name}"),
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ));
    }

    if running {
        let spin = SPINNER[(anim_tick as usize) % SPINNER.len()];
        spans.push(Span::raw("  "));
        spans.push(Span::styled(format!("{spin} {status}"), Style::default().fg(Color::Yellow)));
    } else if !status.is_empty() {
        spans.push(Span::styled(format!("  | {status}"), Style::default().fg(Color::DarkGray)));
    }
    if steer_count > 0 {
        spans.push(Span::styled(format!(" | \u{21b3}steer:{steer_count}"), Style::default().fg(Color::Blue)));
    }
    if queue_count > 0 {
        spans.push(Span::styled(format!(" | queue:{queue_count}"), Style::default().fg(Color::Yellow)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_queue_panel(
    f: &mut Frame,
    area: Rect,
    steer_items: &[String],
    queue_items: &[(i64, String)],
    btns: &mut Vec<QueueBtn>,
) {
    struct E<'a> {
        prefix: &'a str,
        text: &'a str,
        color: Color,
        seq: Option<i64>,
    }
    let mut entries: Vec<E> = Vec::new();
    for s in steer_items {
        entries.push(E { prefix: "\u{21b3} steer", text: s.as_str(), color: Color::Blue, seq: None });
    }
    for (seq, q) in queue_items {
        entries.push(E { prefix: "[queued]", text: q.as_str(), color: Color::Yellow, seq: Some(*seq) });
    }
    let total = entries.len();
    if total == 0 || area.height == 0 {
        return;
    }

    let max_lines = (area.height as usize).min(3);
    let avail_w = area.width as usize;
    let overflow = total > max_lines;
    let item_capacity = if overflow { max_lines.saturating_sub(1) } else { max_lines };
    let start = total.saturating_sub(item_capacity);
    let visible = &entries[start..];

    let mut lines: Vec<Line> = Vec::new();
    if overflow {
        lines.push(Line::from(Span::styled(
            format!(" \u{2191}{} more ", start),
            Style::default().fg(Color::DarkGray),
        )));
    }
    // Clickable queue rows reserve a 6-column trailing control strip
    // (" \u{25b2} \u{25bc} \u{2715}"); steer rows and very narrow terminals
    // render without controls. Each control glyph gets a 1-cell hit rect.
    let btn_w = 6usize;
    for e in visible {
        let clickable = e.seq.is_some() && avail_w > btn_w + 4;
        let cap = if clickable { avail_w.saturating_sub(btn_w) } else { avail_w };
        let head = format!(" {}: {}", e.prefix, e.text);
        let head_display = if head.chars().count() > cap {
            let take = cap.saturating_sub(1);
            format!("{}\u{2026}", head.chars().take(take).collect::<String>())
        } else {
            head
        };
        let head_len = head_display.chars().count();
        let mut spans: Vec<Span> = vec![Span::styled(head_display, Style::default().fg(e.color))];
        if clickable {
            let seq = e.seq.unwrap();
            let y = area.y + lines.len() as u16;
            // Right-align the control strip: pad the head out to `cap` so the
            // glyphs land at the right edge and stay aligned with the hit rects
            // (which `btn_x_offsets` computes from the same right-edge layout).
            let pad = cap.saturating_sub(head_len);
            if pad > 0 {
                spans.push(Span::raw(" ".repeat(pad)));
            }
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

fn place_cursor(f: &mut Frame, composer_area: Rect, input: &str, cursor_idx: usize) {
    let border = 1u16;
    let prompt_w = 2u16;
    let (row, col) = composer::cursor_row_col(input, cursor_idx);
    let x = composer_area.x + border + prompt_w + col as u16;
    let y = composer_area.y + border + row as u16;
    f.set_cursor_position((x, y));
}
