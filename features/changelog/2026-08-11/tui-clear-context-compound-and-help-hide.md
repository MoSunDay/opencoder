# 复合 `/act_clear_context` 命令 + TUI 教程门控修复

## 背景

两个独立但相关的用户体验问题：

1. **复合 clear_context 命令不支持参数**：`split_control_prefix` 对
   `/act_clear_context` 使用精确匹配分支，导致 `/act_clear_context review`
   不被识别为控制命令——"review" 被原样泄漏给 LLM 而不是在清空上下文后作为
   新 prompt 运行。

2. **提交裸控制命令后教程不消失**：TUI 的 in-body 教程仅在
   `chat.blocks.is_empty()` 时显示，但提交裸控制命令（如 `/plan`）不会添加
   transcript block，导致教程在用户已交互后仍然可见。此外，空闲提交后 body
   缓存未及时刷新，教程消失有延迟。

## 变更

### Session 层

- **`crates/session/src/control_cmd.rs`**：`split_control_prefix` 删除
  `/act_clear_context` 的精确匹配分支，改为 head 匹配（与 `/act`、`/plan`
  一致）。`/act_clear_context review` 现在返回 `(ClearContext, Some("review"))`。
- **`crates/session/src/runner/steer.rs`**：`drain_one_queued` 重排后置逻辑——
  compound rest 检查移到 ClearContext 保留计划检查之前，确保
  `/act_clear_context review` 无论是否存在保留计划，都能记录 "review" 作为
  prompt。

### TUI 层

- **`crates/tui/src/chat_types.rs`**：`ChatView` 新增 `submitted: bool` 字段，
  追踪用户是否已提交过至少一次输入。
- **`crates/tui/src/chat.rs`**：`begin_turn()` 置 `submitted = true`。
- **`crates/tui/src/app_loop.rs`**：`TranscriptReset` 保存/恢复 `submitted`。
- **`crates/tui/src/render.rs`**：教程门控条件加 `&& !chat.submitted`。
- **`crates/tui/src/app.rs`**：空闲提交后设 `body_refresh_pending = true`，
  确保 body 缓存立即刷新。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 复合 clear_context 拆分返回 rest | split_clear_context_compound_returns_rest | control_cmd.rs |
| queued 复合 clear_context 返回 Prompt | drain_one_queued_compound_clear_context_returns_prompt | runner/steer.rs |
| 集成：复合 clear_context 运行 rest | clear_context_compound_runs_rest_as_prompt | tests/control_cmd.rs |
| begin_turn 置 submitted | begin_turn_clears_status（扩展断言） | chat_tests/plan_card.rs |
| 复合 clear_context 非 pure | clear_context_with_args_is_not_pure | control_helpers_tests/is_pure_control.rs |
| submitted=true 隐藏教程 | submitted_hides_tutorial_even_with_empty_blocks | render_tests/body.rs |

- 全量回归：`cargo test --workspace` → 2340 passed, 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 0 警告
- build：`cargo build --workspace` → Finished

## Impact Surface

- 用户：`/act_clear_context review` 现在清空上下文并在新上下文中运行 "review"；
  TUI 教程在用户首次提交（包括裸控制命令）后立即消失。
- 不影响：Store trait、LLM 后端、Web SSE 协议、CLI 命令结构。

## Related Docs

- [agents/session](../../agents/session/index.md)
- [agents/tui](../../agents/tui/index.md)
