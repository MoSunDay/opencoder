Commit: (working-tree, 待提交)

# TUI 工具输出后恰好一个空行：对齐 `❯ User:` 块的尾随空行契约

## Context

`❯ User:` 块渲染为「正文 + 恰好一个尾随空行」（`push_user` 推块后紧跟
空 marker，markdown `finish()` 又会裁掉正文尾部空行，故恒为一行）。工具
输出却不是：展开的 Function call 结果与 `!cmd` bash 回显之后经常出现
**两个甚至三个连续空行**，与 User 块视觉不对称。三处根因：

1. 捕获的输出文本自带尾随换行（`sanitize_multiline` 不裁剪尾部，
   `lines()` 会把尾随空行也保留成渲染行）；
2. StepGroup 内最后一个展开 call 自带 per-call 分隔空行，组尾又推
   一个空行，叠加成双空行；
3. 工具收尾 turn 的 `SessionEvent::Done` 空 marker 落在 StepGroup
   自带尾随空行之后，再叠一层。

## Change Summary

- **唯一捕获口**（`chat_helpers.rs::tool_output_lines(text, color)`）：
  sanitize → `lines().take(TOOL_OUTPUT_LINES)` → **裁剪尾随空白行** →
  缩进 2 + 着色，内部空行原样保留。`finish_bash_tool`、`chat.apply`
  的 ToolEnd 捕获、`session_ui/replay.rs` 回放三路全部改走该助手，
  删除各自重复实现。
- **组尾分隔符合并**（`chat_step_render.rs::flatten_step_group`）：
  组内最后一个展开 call 跳过 per-call 分隔空行，由 StepGroup 组尾随
  空行提供唯一空行（`group_final` 判据：`si+1 == steps.len() &&
  ci+1 == step.calls.len()`）。行数记账三处锁步：
  `chat_headers.rs::collect_headers` 与测试镜像
  `chat_tests/line_accounting.rs` 同条件（`ends_on_expanded_call`）。
- **Done marker 条件化**（`chat.rs`）：新增 `last_block_ends_blank()`
  —— 末块已以空行收尾（StepGroup 自带尾随空行、User 块）时跳过 Done
  的空边界 marker，不叠加双空行；纯文本/Thinking 收尾行为不变。

## Impact Surface

- 仅 `crates/tui`：`chat.rs` / `chat_helpers.rs` / `chat_step_render.rs`
  / `chat_headers.rs` / `session_ui/replay.rs` / `chat_tests/*`；
  渲染层与行数记账同步，无接口变更，不影响 Store / session / 模型 context。
- 新增回归：`chat_tests/tool_output_blank.rs` 7 例（组尾 call 恰好一
  空行、非组尾 call 保留分隔、全展开无连续空行、ToolEnd 捕获裁尾、
  内部空行保留、`finish_bash_tool` 裁尾等）；`thinking_state.rs` 1 例
  改判（StepGroup 自带尾随空行后 Done 不再叠 marker）；行数记账镜像
  `line_accounting.rs` 同步收紧。

## Notes / Compatibility

- 历史会话回放（replay）同样生效：捕获路径统一后，旧 transcript 里的
  尾随空行在重放时被裁掉。
- 全量回归：`cargo test --workspace` 253 套件 / 4013 通过 / 0 失败（较基线 +14，含本次 7 例新测试与并行会话用例）。

## Related Docs

- agents/tui/index.md（ChatBlock 条目下新增「工具输出尾随空行契约」）
