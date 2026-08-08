Commit: 05d4bdf110cd7bfa75492f8ea7eebbb7cdb4c662

# TUI 动态文本终端安全边界

## Context

启动/resize 清屏和同步帧分别处理物理 grid 残留与半帧撕裂，
但模型 reasoning、assistant、tool、compaction 和历史回放文本仍可能把 CR、退格、ESC、
C1 等控制字符交给 `Span::raw`。终端会执行这些字符，而 ratatui 仍按普通文本维护 diff
buffer，导致真实 grid 与逻辑 grid 分叉，表现为 Thinking 新旧行持续重叠。

## Change Summary

- 新增纯函数 `terminal_text`，统一移除终端可执行控制字符、展开 TAB，并分别保留多行
  结构或把元数据限制为单行。
- 安全边界放在动态文本进入 `ChatView`、pending mirror 和历史 replay 时；原始事件、
  Store 数据及模型上下文保持不变，旧会话无需迁移即可安全显示。
- 普通文本走 borrowed `Cow` 零分配；流式输出只扫描当前 delta，渲染、viewport cache
  和每帧刷新不扫描完整历史。
- Thinking 长内容切换为短内容的双 buffer 回归确保旧行不会重新出现。

## Impact Surface

- TUI Thinking、Assistant、Tool、Compaction、Subagent、用户 marker、队列面板及顶部元数据。
- Composer 中 TAB 统一展开为四个空格，使光标宽度与终端显示一致。
- 无数据库、配置、环境变量或公开协议变更。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 安全文本零分配借用 | `safe_text_is_borrowed_without_allocation` | `tui/src/terminal_text.rs` |
| 控制字符移除与 TAB 展开 | `multiline_removes_terminal_controls_and_expands_tabs` | `tui/src/terminal_text.rs` |
| Thinking delta 入模前净化 | `dirty_thinking_delta_is_sanitized_before_rendering` | `tui/src/chat_tests/terminal_safety.rs` |
| 历史消息 replay 净化且不改原消息 | `replay_sanitizes_persisted_terminal_controls_without_mutating_message` | `tui/src/session_ui/terminal_safety_tests.rs` |
| 长 Thinking 切短不泄露旧行 | `shorter_thinking_frame_never_reveals_old_lines` / `shorter_frames_keep_vacated_cells_blank_across_buffer_swaps` | `tui/src/render_clear_tests.rs` |

## Gate

- 全量回归：`cargo test --workspace` → **2093 passed / 0 failed**（EXIT=0）。
- 静态检查：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）。
- 构建：`cargo build --workspace` → 成功（EXIT=0）。
- UI 定向：live delta、历史 replay、短帧覆盖及安全文本纯函数测试全部纳入 workspace 回归；原始 Store/模型文本保持不变，仅展示边界净化。

## Related Docs

- [TUI 模块](../../../agents/tui/index.md)
