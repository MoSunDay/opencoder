Commit: (working-tree, pre-initial-commit)

# fix(core/llm/session): make tool & subagent failures visible to the model

## 背景
当工具调用或 subagent 失败时，模型无法感知失败原因，倾向于重复发起同一个
必将失败的调用（doom-loop）。根因有三处互相独立的「失败信号丢失」：

1. **截断只留头部**：工具输出（bash 等）往往把唯一有用的信号——错误行、退出
   码——埋在末尾。旧 `truncate_output_with_error` 只取头部再裁字节，正好把
   尾部错误信息丢掉，模型因此看不到失败原因。
2. **error 标记在 lowering 丢失**：OpenAI `tool` role 没有原生 error 标志。
   旧代码把 `is_error=true` 的工具结果与成功结果用完全相同的 content 下发，
   模型无法区分失败与成功，故可能重复失败调用。
3. **subagent 失败被折叠成不透明横幅**：子会话 `run_loop` 硬失败（LLM 错误、流
   未完成即结束、panic）时，父侧只得到一句 `subagent failed`，无法据真实原因
   反应。

## 变更
### P0-1 工具输出截断改为 head+tail（保留尾部错误信息）
- **`crates/core/src/tool.rs`**：
  - `truncate_output_with_error` 不再 head-only 截断，改为先按行 head+tail
    裁剪、再对结果按字节 head+tail 裁剪；两条预算都命中时拼接两段原因。
  - 新增私有辅助 `head_tail_lines`（按行预算保留首尾，中间用
    `[N lines omitted]` 标记）与 `head_tail_bytes`（按字节预算保留首尾，
    `[N bytes omitted]`，两侧均走 char boundary 保证合法 UTF-8）。
  - 公开签名与截断标记契约（`[output truncated, original ...]`）不变，
    被 bash/read/grep/glob/ls/web_fetch/web_search/ssh_pty/chrome_headless/
    computer_use 等 10 个工具复用。

### P0-2 error 工具结果在 lowering 时加 `[error]` 前缀（Role::Tool 路径）
- **`crates/llm/src/message.rs`**：
  - 新增 `tool_result_body(content, is_error)`：`is_error` 为真时输出
    `[error] {content}`，否则原样返回（非错误内容字节不变）。
  - `push_tool_results`（`Role::Tool` 下降路径）改用它渲染 content。

### P1-1 subagent 失败时上报真实错误原因
- **`crates/session/src/runner.rs`**：
  - `run_subagent` 的失败分支不再返回固定 `subagent failed`：当子会话返回
    `Err(e)` 时拼出 `subagent failed: {e}`（并有子产物文本则追加）；
    `Ok` 但子文本为空才回退到通用横幅。父模型据此可对真实原因反应。

### P2 User-role 内嵌的 error 工具结果也加 `[error]` 前缀
- **`crates/llm/src/message.rs`**：
  - `push_user`（`Role::User` 下降路径）同样改用 `tool_result_body`，
    使搭载于 User 消息上的 `ToolResult(is_error=true)` 也被正确标记。

## 测试覆盖
| 功能（优先级） | 测试名 | 文件 |
|------|--------|------|
| P0-1 head+tail 行截断保留尾部错误 | `truncate_output_preserves_tail_error_text` | crates/core/tests/tool_filter.rs |
| P0-1 head+tail 行截断同时保留首尾 | `truncate_output_preserves_both_head_and_tail` | crates/core/tests/tool_filter.rs |
| P0-1 head+tail 字节截断保留尾部 | `truncate_output_bytes_preserve_tail` | crates/core/tests/tool_filter.rs |
| P0-2 error 结果被 `[error]` 前缀 | `error_tool_result_is_prefixed_in_lowering` | crates/llm/tests/lower_messages.rs（新增） |
| P0-2 成功结果不被前缀（字节不变） | `ok_tool_result_is_not_prefixed_in_lowering` | crates/llm/tests/lower_messages.rs（新增） |
| P1-1 subagent 失败上报真实错误 | `subagent_failure_surfaces_actual_error` | crates/session/tests/subagent.rs |
| P2 User-role 内嵌 error 结果被前缀 | `user_role_error_tool_result_is_also_prefixed` | crates/llm/tests/lower_messages.rs（新增） |

- 全量回归：`cargo test --workspace` → **887 passed; 0 failed; 0 ignored**
- clippy：`cargo clippy -p opencoder-core -p opencoder-llm -p opencoder-session --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 干净编译
- 行数：tool.rs (178)、message.rs (133)、lower_messages.rs (65)、tool_filter.rs (199)
  均 < 800；**runner.rs 1207 行为既有超标债务（触改前 1185，本次 +23 属最小必要修复）**，
  建议另开 follow-up 按 subagent-dispatch / steer-drain / run_loop 拆子模块。

## Impact Surface
- core/tool：截断策略变化，影响所有 10 个工具的长输出；截断标记契约保持不变。
- llm/message：模型下降边界——error 工具结果内容多出 `[error] ` 前缀（6 字节）；
  非错误内容字节不变（已由 `ok_tool_result_is_not_prefixed_in_lowering` 断言）。
- session/runner：subagent 失败时 task 工具结果内容更详细（含真实错误原因）。
- 不影响：CLI / Web / Store 契约，无新增 SessionEvent 变体 / Tool / HTTP 端点。

## Related Docs
- [agents/core](../../agents/core/index.md)
- [agents/llm](../../agents/llm/index.md)
- [agents/session](../../agents/session/index.md)
