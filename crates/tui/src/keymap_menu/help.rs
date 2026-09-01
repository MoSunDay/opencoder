//! Static help overlay rendered on top of the keymap modal.
//! Opened via the "帮助" button in the keymap modal's button bar.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::composer;
use crate::theme;

/// Full shortcut reference text (Chinese), shown in the help overlay.
pub const HELP: &str = "\
快捷键列表：

  Shift+Tab        清空上下文并接续执行 (/act_clear_context)：保留最后回复作
                   上下文；先倒计时确认，Esc 回撤 / Enter 提前执行
                   输入框有内容时作为附加需求一并提交（运行中则排队，空闲边界生效）
  Enter            提交（空闲） / 转向（运行中，下一轮生效）
  Tab              提交（空闲） / 排队跟进（运行中，完成后提交）
  Ctrl+V          粘贴剪贴板图片（截图）
  Alt+回车            插入换行（多行输入）
  $                选择并插入技能 -> $name；提交时加载
  /                命令选择: /task（会话）, /config（设置）, /model（模型）, /compact（压缩）
                   /plan（只读探索） /act（执行） /act_clear_context（清空上下
                   文，倒计时确认）
  Shift+I          编辑计划（plan 模式、空闲时）: i/a 编辑, :wq 保存, :q! 放弃
  Ctrl+G          复制模式: 交还终端原生拖拽选择（正文去装饰全宽显示）, 终端快捷键复制, Esc/Ctrl+G 退出
  Esc              关闭帮助/弹窗/清空输入
  Esc Esc          双击 Esc 中断运行中的任务
  Ctrl+C          中断运行中的任务（同 Esc Esc）
  Ctrl+D           退出
  Ctrl+H           快捷键设置面板（查看 / 重绑快捷键，含「恢复默认」与「帮助」按钮）
  Ctrl+W           删除光标前的单词
  Alt+F / Alt+B    光标向前/向后移动一个单词（readline 风格）
  Ctrl+U           清空整个输入行（可被 Ctrl+Z 撤销）
  Ctrl+A / Ctrl+E  光标移到行首/行尾
  Ctrl+Z / Ctrl+Y  撤销 / 重做输入编辑
  ↑ / ↓            多行时移动光标；单行时浏览历史记录
  PageUp/Down      滚动对话记录  （PageDown = 跳到底部）
  Shift+PageUp/Down 滚动转向面板（查看更早的排队条目 / 回到最新）
  Ctrl+F           强制重新渲染屏幕
  Ctrl+L           退出子代理视图 / 折叠所有输出 / 回到底部跟随 / 清空输入
  Ctrl+L / Ctrl+U  /config、/model 弹窗内: 清空当前聚焦字段

鼠标:            滚轮滚动对话记录；点击箭头跟随最新
                  文本复制: Ctrl+G 进入复制模式（暂停鼠标捕获 + 正文去装饰全宽显示），
                  对话正文可用 ↑/↓ 逐行、PageUp 向前翻页、PageDown/End 回最新、Home 到开头；
                  用终端自带拖拽选择和复制快捷键取干净文本; Esc/Ctrl+G 退出恢复
                  SHIFT+拖拽 = 终端原生选择（Kitty/WezTerm 等透传 Shift 的终端）
                  转向面板: ✕ 删除, > 立即提交（中断并提升）
";

/// Word-wrap a single source line into multiple display lines that each fit
/// `max_w` display columns. Uses `composer::char_width` so CJK / wide chars
/// are handled correctly. Breaks at the last space before overflow; if a
/// single word exceeds `max_w` it is hard-broken.
pub(crate) fn wrap_line(text: &str, max_w: usize) -> Vec<String> {
    if max_w == 0 {
        return vec![text.to_string()];
    }
    let mut result: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_w: usize = 0;
    let mut last_space: Option<usize> = None; // byte offset in `current`

    for ch in text.chars() {
        let cw = composer::char_width(ch);
        if ch == ' ' {
            last_space = Some(current.len());
        }
        if current_w + cw > max_w && !current.is_empty() {
            if let Some(sp) = last_space {
                let remainder: String = current[sp..].trim_start().to_string();
                let head = current[..sp].trim_end().to_string();
                result.push(head);
                current = remainder;
                current_w = current.chars().map(composer::char_width).sum();
            } else {
                result.push(std::mem::take(&mut current));
                current_w = 0;
            }
            last_space = None;
            if ch == ' ' {
                continue;
            }
        }
        current.push(ch);
        current_w += cw;
    }
    if !current.is_empty() {
        result.push(current);
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

/// Build the wrapped help lines from the HELP constant, fitting `max_w`
/// display columns per line.
fn build_wrapped_lines(max_w: usize) -> Vec<String> {
    HELP.lines()
        .flat_map(|line| wrap_line(line, max_w))
        .collect()
}

/// Render the help popup centered on `area`, scrolled by `scroll` lines.
/// Designed to be called *after* the keymap popup so it appears on top.
pub fn render_help_overlay(f: &mut Frame, area: Rect, scroll: u16) {
    let h = 22u16.min(area.height.saturating_sub(2));
    let w = 62u16.min(area.width.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);
    f.render_widget(Clear, popup);

    let inner_w = (w.saturating_sub(2) as usize).max(1);
    let wrapped = build_wrapped_lines(inner_w);
    let lines: Vec<Line> = wrapped
        .iter()
        .map(|s| {
            Line::from(Span::styled(
                s.as_str(),
                Style::default().fg(theme::subtle()),
            ))
        })
        .collect();

    let block = crate::theme::rounded_block_focus("帮助 (Esc 关闭, ↑↓ 滚动)");
    f.render_widget(
        Paragraph::new(lines).scroll((scroll, 0)).block(block),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_line_short_passthrough() {
        let result = wrap_line("hello", 80);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn wrap_line_breaks_at_space() {
        let result = wrap_line("aaa bbb ccc", 7);
        assert_eq!(result, vec!["aaa bbb", "ccc"]);
    }

    #[test]
    fn wrap_line_long_word_hard_break() {
        let result = wrap_line("abcdefghij", 4);
        assert!(result.len() > 1);
        for line in &result {
            assert!(line.chars().count() <= 4);
        }
    }

    #[test]
    fn wrap_line_cjk_aware() {
        let result = wrap_line("你好世界", 4);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], "你好");
        assert_eq!(result[1], "世界");
    }

    #[test]
    fn wrap_line_empty() {
        let result = wrap_line("", 80);
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn build_wrapped_lines_nonempty() {
        let lines = build_wrapped_lines(40);
        assert!(!lines.is_empty());
    }

    #[test]
    fn help_copy_mode_text_is_current() {
        // Ctrl+G hands the drag back to the terminal and renders the body
        // undecorated; copying is the terminal's own shortcut (the app
        // never writes the clipboard), so the in-app OSC52 selection
        // wording must not return.
        assert!(HELP.contains("终端原生拖拽选择"));
        assert!(HELP.contains("去装饰"));
        assert!(HELP.contains("PageDown/End 回最新"));
        assert!(!HELP.contains("OSC52"));
        assert!(!HELP.contains("应用内选择模式"));
    }

    #[test]
    fn help_matches_plan_world() {
        // The plan/act dual-mode key machinery is gone: no Ctrl+T / Alt+Tab /
        // Ctrl+Shift+Tab mode toggles may be advertised. Shift+Tab is the
        // clear-context submit, and the slash menu documents /plan.
        assert!(!HELP.contains("切换 plan / act"));
        assert!(!HELP.contains("仅切换模式"));
        assert!(HELP.contains("/act_clear_context"));
        assert!(HELP.contains("/plan"));
        assert!(HELP.contains("Shift+I"));
    }

    #[test]
    fn help_no_stale_hide_composer_shortcut() {
        // hide_composer（折叠/展开底部输入框）功能已从代码中完全移除，
        // 帮助页不得再宣传该快捷键，误导用户按键无响应。
        // 按首 token 精确匹配，避免误伤合法的 "Ctrl+Shift+Tab" 前缀。
        assert!(!HELP
            .lines()
            .any(|l| l.split_whitespace().next() == Some("Ctrl+Shift+T")));
        assert!(!HELP.contains("折叠/展开"));
        // 邻行防误删：Ctrl+G 复制模式仍在。
        assert!(HELP.contains("Ctrl+G"));
    }

    #[test]
    fn help_mentions_keymap_panel() {
        // The updated text should reference the keymap settings panel, not the
        // old "open/close this help" phrasing.
        assert!(HELP.contains("快捷键设置面板"));
        assert!(!HELP.contains("打开/关闭此帮助"));
    }
}
