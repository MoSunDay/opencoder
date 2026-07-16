# OSC52-only 复制状态消息纠正 + tmux 提示

## 背景

拖选复制在「无本地剪贴板命令」时显示 `⚠ No clipboard tool found — OSC52 only`。问题：OSC52 转义序列**实际上已发送成功**，但警告色（⚠）让人误以为复制失败。常见触发场景是 tmux/screen 拦截或 VTE 终端默认禁用 OSC52——此时 OSC52 到达终端但被丢弃，而应用层无从感知。

## 变更

### TUI：OSC52-only 消息从警告改为信息（`crates/tui/src/selection.rs`）
- `CopyReport::status_message()` 的 `None`（无本地工具）分支不再返回 `⚠ No clipboard tool found — OSC52 only`，改为调用新 `osc52_only_message()`：`📋 Copied N line(s) via OSC52`——信息色而非警告色，如实反映「OSC52 已发出」。
- 新增 `under_tmux()`：检测 `TMUX` 环境变量。tmux 默认静默丢弃 OSC52 序列，是「OSC52 发了但粘贴为空」的首要原因。
- 当处于 tmux 下，消息追加可操作提示：`— no paste? tmux: set -g set-clipboard on`，让用户一步定位问题。
- `osc52_only_message(lines, under_tmux)` 把 tmux 标志作为参数传入，使两个分支均可纯单元测试（不触碰真实环境变量）。
- **file:line**: `selection.rs:45-75`（status_message + osc52_only_message + under_tmux）。

## 测试清单

| 功能 | 测试名 | 文件 |
|------|--------|------|
| OSC52-only 消息含行数且无警告 | `osc52_only_message_includes_line_count_and_no_warning` | `crates/tui/src/selection.rs` |
| OSC52-only 消息在 tmux 下加提示 | `osc52_only_message_adds_tmux_hint` | `crates/tui/src/selection.rs` |
| 无本地工具状态消息纠正（更新） | `copy_report_status_without_local_tool` | `crates/tui/src/selection.rs` |

## Impact Surface
- **用户可感知**：拖选复制不再显示误导性警告；tmux 用户直接看到 `set-clipboard on` 提示，减少排障时间。
- **不影响**：OSC52 转义序列发送逻辑、本地剪贴板回退链、Store/ChatStream 抽象边界。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
