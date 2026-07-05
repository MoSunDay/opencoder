Commit: (working-tree, pre-initial-commit)

# 迭代三：TUI 大改（4-region 布局 / scrollback / 可见光标 / 上下文显示 / subagent / steer-followup TUI 接入）

## Context
迭代二完成了存储 / 配置 / steer / web / 恢复 / 性能，但 TUI 仍是迭代一的简单三行布局：无 scrollback、光标不可见、不显示上下文用量、不渲染 subagent 事件、steer/followup 仅在 web 层可用。本迭代对齐 codex/opencode 的 TUI 布局参考，同时保持 Rust 原生高性能（ratatui + crossterm，不移植 Effect-TS/flexbox 实现）。

## Change Summary
- **4-region 布局**：`app.rs` 重写为 header(1) / body(Min) / composer(3) / status(1) 四区域 `Layout::vertical`；header 显示 model + agent + workdir + 上下文百分比；status 显示 running/idle + steer/queue 计数 + chat.status。
- **上下文显示**：header 实时显示 `ctx N% (used/limit)`，颜色随使用率变化（<60% 绿 / 60–84% 黄 / ≥85% 红）；`fmt.rs` 提供 `format_tokens_compact`（K/M/B/T 后缀 + 紧凑）和 `context_percent`（baseline 偏移 + clamp）。
- **可见光标**：启动时 `SetCursorStyle::SteadyBar`；每帧 `Frame::set_cursor_position` 放置光标到 composer 输入位置；`composer.rs` 提供 unicode 宽度感知的光标列计算（CJK 双宽）。
- **Scrollback**：body 区 `Paragraph::scroll` + `PageUp/Down` + `Ctrl+U/D` 滚动；auto-follow 到底部（用户上滚时暂停跟随）。
- **光标移动**：`Left/Right/Home/End` 移动字符光标；`Backspace` 删除前一个字符（unicode 安全）；`composer.rs` 提供 `insert_char` / `backspace` / `cursor_column` 纯函数。
- **subagent 渲染**：`SessionEvent` 新增 `SubagentStart{id,kind,prompt}` / `SubagentEnd{id,ok,summary}`；`run_subagent` 转发子 agent 事件到父级 `on_event`；TUI / CLI / web 三处 `match` 均已覆盖。
- **TUI steer / followup**：TUI 会话挂载 libsql Store（与 CLI/web 同路径）；`Ctrl+O` admit steer（运行中重定向）、`Ctrl+J` admit follow-up 到队列；worker 的 drain loop（迭代二）自动在 turn 边界 / idle 消费——TUI 现已具备 opencode 缺失的 steer 能力。
- **keybind help**：更新为完整快捷键列表（含 steer/queue/scroll/cursor）。

## Impact Surface
- 新增文件：`crates/tui/src/fmt.rs`（74 行）、`crates/tui/src/composer.rs`（118 行）。
- 重写文件：`crates/tui/src/app.rs`（268→472 行）、`crates/tui/src/chat.rs`（重写含 scroll + subagent）、`crates/tui/src/keybind.rs`。
- `SessionEvent` 枚举新增两个 variant → 所有 `match SessionEvent` 处（TUI chat.rs / web handle.rs / CLI run.rs）均已补齐。
- 新增依赖：`dirs = "5"`（TUI crate，用于 data_local_dir）。
- workspace 测试从 50 增至 58（+8 TUI 纯函数测试：fmt 4 + composer 4）。

## Verification
- `cargo clippy --workspace --all-targets -- -D warnings`：全绿。
- `cargo test --workspace`：58 passed / 0 failed。
- PTY smoke test（80×24）：header / ctx display / prompt / agent label / alt screen / bar cursor / no panic 全 PASS。

## Notes / Compatibility
- TUI Store 与 CLI/web 共享同一路径（`data_local_dir()/opencode/{hash}/opencode.db`），但不同 entry 可能 hash 不同 → 不同 workdir 的 session 独立。
- 上下文用量为增量估算（TextDelta / ReasoningDelta / ToolEnd / SubagentEnd 累加 estimate，Compaction 重置），非精确 token——status bar 用途足够。
- ratatui `Paragraph::scroll` 以行为单位，长行 wrap 时 scroll 可能略偏——MVP 可接受，后续可引入 `ScrollState`。

## Related Docs
- [agents/session](../../../agents/session/index.md) —— SessionEvent / drain loop
- [agents/store](../../../agents/store/index.md) —— Store trait / SessionInput
