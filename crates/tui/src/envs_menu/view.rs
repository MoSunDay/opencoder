//! Rendering for the `/envs` modal (mirrors `/mcp` view patterns).

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::form::{EnvField, EnvNameForm};
use super::list::EnvsList;
use super::state::EnvsMenu;

/// Dispatch to the correct renderer based on the modal variant.
pub fn render_envs_popup(f: &mut Frame, area: Rect, composer_top: u16, menu: &EnvsMenu) {
    match menu {
        EnvsMenu::List(list) => render_list(f, area, composer_top, list),
        EnvsMenu::Form(form) => render_form(f, area, composer_top, form),
    }
}

fn focus_style() -> Style {
    Style::default()
        .fg(crate::theme::warn_color())
        .add_modifier(Modifier::BOLD)
}

fn dim_style() -> Style {
    Style::default().fg(crate::theme::subtle())
}

fn val_style() -> Style {
    Style::default().fg(crate::theme::text())
}

// ── env list ───────────────────────────────────────────────────────────────

fn render_list(f: &mut Frame, area: Rect, composer_top: u16, list: &EnvsList) {
    let n = list.envs.len() as u16;
    let want_h = n.max(3) + 5;
    let h = want_h.min(20u16).min(composer_top.max(1));
    let w = 68u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = composer_top.saturating_sub(h);
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let title = match list.confirm_delete {
        Some(_) => " /envs \u{2014} CONFIRM DELETE? y=delete, n/Esc=cancel ".to_string(),
        None => {
            " /envs \u{2014} Enter=switch, n=new, e=recapture, d=delete, Esc=close ".to_string()
        }
    };
    let block = crate::theme::rounded_block_plain().title(title);

    let mut lines: Vec<Line> = Vec::new();
    // row 0: <base> (no env layer)
    let base_selected = list.selected == super::list::BASE_ROW;
    let base_active = list.active.is_none();
    let base_style = if base_selected {
        focus_style()
    } else if base_active {
        Style::default().fg(crate::theme::accent())
    } else {
        dim_style()
    };
    let mut base = format!(
        " {} <base> \u{2014} \u{57fa}\u{7840}\u{914d}\u{7f6e} (\u{4e0d}\u{542f}\u{7528} env)",
        if base_selected { "\u{276f}" } else { " " }
    );
    if base_active {
        base.push_str("  [active]");
    }
    lines.push(Line::styled(base, base_style));

    for (i, name) in list.envs.iter().enumerate() {
        let row = i + 1;
        let selected = row == list.selected;
        let confirming = list.confirm_delete == Some(row);
        let is_active = list.active.as_deref() == Some(name.as_str());
        let style = if confirming {
            Style::default()
                .fg(ratatui::style::Color::Red)
                .add_modifier(Modifier::BOLD)
        } else if selected {
            focus_style()
        } else if is_active {
            Style::default().fg(crate::theme::accent())
        } else {
            val_style()
        };
        let mut label = format!(" {} {}", if selected { "\u{276f}" } else { " " }, name);
        if is_active {
            label.push_str("  [active]");
        }
        lines.push(Line::styled(label, style));
    }
    if list.envs.is_empty() {
        lines.push(Line::styled(
            " (no envs \u{2014} press 'n' to capture one)",
            dim_style(),
        ));
    }
    // bottom: selected target + reminder that activation redirects saves
    let target = match list.selected_env() {
        Some(name) => format!(" \u{2192} ~/.opencoder/envs/{name}/ (\u{6fc0}\u{6d3b}\u{540e}\u{914d}\u{7f6e}\u{6539}\u{52a8}\u{9ed8}\u{8ba4}\u{5199}\u{5165}\u{6b64}\u{5904}\u{ff1b}\u{9879}\u{76ee}\u{5c42}\u{5df2}\u{6709}\u{53ef}\u{7f16}\u{8f91}\u{914d}\u{7f6e}\u{65f6}\u{4ecd}\u{5199}\u{9879}\u{76ee}\u{5c42})"),
        None => " \u{2192} \u{57fa}\u{7840}\u{914d}\u{7f6e} (~/.opencoder + \u{9879}\u{76ee}\u{5c42})".to_string(),
    };
    lines.push(Line::styled(target, dim_style()));

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left)
            .wrap(ratatui::widgets::Wrap::default()),
        popup,
    );
}

// ── name form ──────────────────────────────────────────────────────────────

const LABEL_W: usize = 15;

fn render_form(f: &mut Frame, area: Rect, composer_top: u16, form: &EnvNameForm) {
    let h = 8u16.min(composer_top.max(1));
    let w = 60u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = composer_top.saturating_sub(h);
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);
    let block = crate::theme::rounded_block_plain()
        .title(" /envs \u{2014} new env (Enter=create, Tab=field, Esc=back) ");

    let mut lines: Vec<Line> = Vec::new();
    let name_focused = form.field == EnvField::Name;
    lines.push(Line::from(vec![
        Span::styled(format!(" {:<width$}", "name", width = LABEL_W), dim_style()),
        Span::styled(
            form.name.clone(),
            if name_focused {
                focus_style()
            } else {
                val_style()
            },
        ),
    ]));
    let cap_focused = form.field == EnvField::Capture;
    lines.push(Line::from(vec![
        Span::styled(
            format!(" {:<width$}", "capture", width = LABEL_W),
            dim_style(),
        ),
        Span::styled(
            if form.capture {
                "[x] \u{5f53}\u{524d}\u{57fa}\u{7840}\u{914d}\u{7f6e}\u{5feb}\u{7167}"
            } else {
                "[ ] \u{7a7a}\u{76ee}\u{5f55}"
            },
            if cap_focused {
                focus_style()
            } else {
                val_style()
            },
        ),
    ]));
    match form.validation_error() {
        Some(err) => lines.push(Line::styled(
            format!(" ! {err}"),
            Style::default().fg(crate::theme::err_color()),
        )),
        None => lines.push(Line::styled(
            " \u{5408}\u{6cd5}\u{540d}\u{79f0}\u{ff0c}Enter \u{521b}\u{5efa}",
            dim_style(),
        )),
    }
    lines.push(Line::styled(
        " capture=\u{542b}\u{9879}\u{76ee}\u{5c42}\u{7684}\u{57fa}\u{7840}\u{94fe}\u{5feb}\u{7167}(\u{4e0d}\u{542b} env-var \u{8986}\u{76d6})",
        dim_style(),
    ));

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Left)
            .wrap(ratatui::widgets::Wrap::default()),
        popup,
    );
    if name_focused {
        let col = crate::composer::cursor_column(&form.name, form.name_cursor);
        f.set_cursor_position((
            (popup.x + 1 + LABEL_W as u16 + col).min(popup.right().saturating_sub(1)),
            popup.y + 1,
        ));
    }
}
