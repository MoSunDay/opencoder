//! Tutorial text rendered inside the body region when a session has no
//! blocks yet. It disappears automatically once the first prompt is
//! submitted (blocks become non-empty), so no key is required to dismiss it.

use crate::theme;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

/// The tutorial text shown in the body when the session is empty.
const TUTORIAL: &str = "\
  👋 欢迎使用 OpenCoder！

  🤖 Rust 原生的 AI 编码助手。在下方输入框中开始提问吧！

  🎮 常用操作：
    • Alt+回车  换行（多行输入）
    • Ctrl+T  保留上下文，在 act / plan 间切换
    • Shift+Tab  保留计划、切到 act 并开始执行（倒计时确认，Esc 回撤）
    • /  命令菜单（/plan 只读探索、/act 执行、会话切换、设置等）
    • $  选择并插入技能
    • Ctrl+F  强制重绘屏幕（花屏/乱码时按一下）
    • Ctrl+H  打开快捷键设置

  💡 开始对话后本教程自动消失，开启你的编码之旅吧！
";

const REMOTE_TUTORIAL: &str = "\
  👋 Server 任务已准备好，在下方输入框中开始提问。

  🎮 常用操作：
    • 回车  发送；执行中回车追加指令，Tab 排队
    • Alt+回车  换行
    • 连按两次 Esc 或 /stop  打断当前执行
    • /agent  选择 Server 注册的 Agent / Operator 并新建任务
    • /agent self  新建本地任务，进入空白上下文
    • /task  查看、切换和恢复任务
    • Ctrl+F  强制重绘屏幕

  💡 切换任务或退出 TUI 后，Server 任务仍会继续执行。
";

pub fn render_remote_tutorial_in_body(f: &mut Frame, inner: Rect) {
    render_text(f, inner, REMOTE_TUTORIAL);
}

/// Render the tutorial directly inside `inner` (the body's inner area).
/// No overlay/popup: the text lives within the normal body block and is
/// replaced by real conversation content as soon as the first block appears.
pub fn render_tutorial_in_body(f: &mut Frame, inner: Rect) {
    render_text(f, inner, TUTORIAL);
}

fn render_text(f: &mut Frame, inner: Rect, text: &str) {
    let header_st = Style::default()
        .fg(theme::ok_color())
        .add_modifier(Modifier::BOLD);
    let op_st = Style::default().fg(theme::accent());
    let hint_st = Style::default().fg(theme::muted());
    let lines: Vec<Line> = text
        .lines()
        .map(|s| {
            if s.contains('\u{2022}') {
                Line::from(Span::styled(s, op_st))
            } else if s.contains('\u{1f4a1}') {
                Line::from(Span::styled(s, hint_st))
            } else if s.trim().is_empty() {
                Line::from(Span::raw(s))
            } else {
                Line::from(Span::styled(s, header_st))
            }
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .wrap(Wrap { trim: false }),
        inner,
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn remote_welcome_shows_server_controls_and_detach_behavior() {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|f| super::render_remote_tutorial_in_body(f, f.area()))
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        for expected in ["/agent self", "/task", "/stop", "Server"] {
            assert!(text.contains(expected));
        }
        assert!(!text.contains("Ctrl+T"));
    }
}
