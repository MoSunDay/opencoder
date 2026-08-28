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
    • Shift+Tab  清空上下文并接续执行（保留最后回复作上下文）
    • /  命令菜单（/sandbox 只读探索、/act 执行、会话切换、设置等）
    • $  选择并插入技能
    • Ctrl+F  强制重绘屏幕（花屏/乱码时按一下）
    • Ctrl+H  打开快捷键设置

  💡 开始对话后本教程自动消失，开启你的编码之旅吧！
";

/// Render the tutorial directly inside `inner` (the body's inner area).
/// No overlay/popup: the text lives within the normal body block and is
/// replaced by real conversation content as soon as the first block appears.
pub fn render_tutorial_in_body(f: &mut Frame, inner: Rect) {
    let header_st = Style::default()
        .fg(theme::ok_color())
        .add_modifier(Modifier::BOLD);
    let op_st = Style::default().fg(theme::accent());
    let hint_st = Style::default().fg(theme::muted());
    let lines: Vec<Line> = TUTORIAL
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
