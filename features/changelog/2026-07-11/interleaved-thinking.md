Commit: (working-tree)

# 交错思考 (Interleaved Thinking)：reasoning_content 持久化与回传

## Context

OpenAI function-call 协议下，模型在工具调用 turn 产生的 `reasoning_content` 需要持久化到 assistant 消息并在后续请求中原样传回，使模型能在工具结果之间继续推理（交错思考）。**DeepSeek-V4 强制要求此行为**——tool-call turn 不回传 `reasoning_content` 会返回 HTTP 400。GLM-5.2 同走 OpenAI 兼容协议。

此前代码的关键缺口：runner 实时推送 `ReasoningDelta` 到 UI（显示），但**从不持久化** reasoning 到 assistant message——`push_assistant` 的 `reasoning_content` 序列化逻辑（`crates/llm/src/message.rs:82-84`）是死代码。

## Change Summary

- **core：Config 增 `interleaved_thinking: Option<bool>`**（`crates/core/src/config.rs`，默认 `Some(true)`）；`merge_into` 解析；`has_editable_key` 含之。
- **session：runner 捕获 reasoning**（`crates/session/src/runner.rs`）——`run_one_llm_call` 新增 `reasoning_buf`，从 `LlmEvent::ReasoningDelta` 累积，返回值由 3-tuple 改为 4-tuple `(text, reasoning, tool_calls, usage)`。
- **session：runner 持久化 Reasoning 块**——`run_loop` 在 `interleaved_thinking.unwrap_or(true)` 且 `tool_calls` 非空且 `reasoning` 非空时，在 Text 之前插入 `ContentBlock::Reasoning { text }`。非工具 turn 不持久化（API 会忽略，省 token）。
- **llm：出站回传无需改动**——`push_assistant`（`crates/llm/src/message.rs:82-84`）已从 `ContentBlock::Reasoning` 序列化 `reasoning_content`。runner 一旦创建该块即自动回传。
- **tui：`/model` 菜单新增 interleave on/off 切换行**（`crates/tui/src/model_menu/{state,view}.rs`）——`Field::InterleavedThinking` 插入 Reasoning 与 Threshold 之间；↑↓←→/Space 切换；popup 高度 16→17。
- **cli：`opencode models` 增 interleave 行**（`crates/cli/src/session_cmd.rs`）。

## Impact Surface
- 新增文件：`crates/session/tests/interleaved_thinking.rs`（4 测试）。
- 修改：`crates/core/src/config.rs`、`crates/core/tests/config_contract.rs`（+4 测试）、`crates/session/src/runner.rs`、`crates/cli/src/session_cmd.rs`（+2 测试）、`crates/tui/src/model_menu/{state,view,mod}.rs`。
- 行为契约：默认开启 interleaved thinking（reasoning_content 在工具调用 turn 持久化并回传）；`interleaved_thinking: false` 关闭则恢复旧行为（reasoning 仅显示不回传）。

## Notes / Compatibility
- DeepSeek-V4 要求 tool-call turn 回传 `reasoning_content`，否则 HTTP 400——本变更使默认行为合规。
- 非工具 turn 的 reasoning 不持久化（DeepSeek 文档：会被 API 忽略），避免无谓 token 开销。
- 不发送 DeepSeek 专有 `{"thinking":{"type":"enabled"}}` 字段（DeepSeek thinking 默认 enabled）。
- 全量回归：`cargo test --workspace` 全绿（含 10 新增测试）；`cargo clippy --workspace --all-targets -- -D warnings` 零警告。
- 行数 gate：config.rs 388、runner.rs 618、state.rs 383、view.rs 82（均 ≤800）。
