# TUI 渲染提速（输入框 30fps / 消息区 3fps）+ 滚轮上加快 + Resume 悬空 tool_use 修复

## 背景

三项独立改进合并提交：

1. **FPS**：输入框（`FRAME_MS`）此前 100ms/帧（10 FPS），打字与光标有可感延迟；消息正文缓存（`BODY_REFRESH_MS`）200ms（5 FPS）。输入框提到 30 FPS（33ms），正文降到 3 FPS（333ms）——把刷新预算从「全量 10 FPS」重分配到「输入框高频、正文低频」，输入响应更跟手、正文 token 风暴时 CPU 更低。
2. **滚动**：鼠标滚轮向上每格仅 3 行，长会话往回翻很慢；提到 8 行/格（向下保持 3 行不变）。
3. **Resume 修复**：硬中断（kill -9 / 崩溃）发生在 assistant 持久化了 `tool_use` 但尚未持久化对应 `tool_result` 之间时，恢复后的 transcript 以悬空 `tool_use` 结尾。下一次 LLM 调用因每个 `tool_use` 缺少匹配的 `tool_result` 而被大多数 OpenAI 兼容后端以 HTTP 400 拒绝。

## 变更

### `crates/tui/src/app.rs`
- `FRAME_MS` `100` → `33`（30 FPS，输入框/渲染上限）。`BODY_REFRESH_MS` `200` → `333`（3 FPS，正文缓存重建）。`ANIM_TICK_MS`（spinner，10 FPS）与 `MODE_FLASH_TICKS` 保持不变。
- 同步全部相关注释（10/5 FPS → 30/3 FPS）。

### `crates/tui/src/app_helpers.rs`
- `MouseEventKind::ScrollUp` 步长 `saturating_sub(3)` → `saturating_sub(8)`。`ScrollDown`（`add(3)`）不动——按需求仅加快向上。

### `crates/session/src/resume.rs`
- compaction trim 之后、构建 `SessionState` 之前，扫描 messages：先收集所有已有匹配 `tool_result` 的 `tool_use_id`（`HashSet`），再找出每个无匹配的 `ToolUse`，为它们合成单条 `Role::Tool` 消息（每块一个 `ToolResult { is_error: true, content: "session interrupted: tool result missing" }`，`synthetic: true`）。
- 持久化（`store.append_message`，使后续 resume 看到合规 transcript）+ 入内存 `messages`（在 `let n = messages.len()` 之前，保持 `persisted_count` 一致）。
- 仅对真正悬空的调用注入；已有匹配结果的不重复注入。`warn!` 日志记录注入数量。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 滚轮上每格 8 行（旧 3 行）且脱离 follow | `scrollup_advances_faster_than_default` | `crates/tui/src/app_helpers.rs`（内联 unit，新增） |
| 悬空 tool_use 恢复后合成 error tool_result（内存 + 持久化） | `resume_synthesizes_error_result_for_dangling_tool_use` | `crates/session/tests/resume_reconcile.rs`（integration，新增） |
| 已配对 tool_use 不被重复注入合成结果（消息数不变、store 无新增行） | `resume_does_not_inject_when_tool_result_already_present` | `crates/session/tests/resume_reconcile.rs`（integration，新增） |

回归：`cargo test --workspace` → 567 passed / 0 failed（基线 564 + 本轮新增 3）。
