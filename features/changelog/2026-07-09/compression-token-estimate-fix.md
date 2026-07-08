Commit: (working-tree, pre-initial-commit)

# context 压缩不触发根修 — token 估算漏算 ToolResult/ToolUse/Reasoning

## 背景
用户设置了 `context_threshold: 100000`，但压缩从未触发。状态栏显示 ctx >100k，压缩却纹丝不动。

## 根因
`estimated_tokens()` → `estimate_messages()` → `Message::text()` 只过滤 `ContentBlock::Text`。
`ToolResult.content`（文件读取、grep/bash 输出——占真实 context 90%+）、`ToolUse.input` JSON、`Reasoning` 全部被忽略。
一个真实 120k token 的会话估算只有 ~3k，`3k >= 100k` 永远不成立。

同时 TUI 状态栏用**另一个**计数器 `track_context`（`app.rs`），它**确实**累加 ToolEnd output——所以用户看到 >100k，但压缩阈值检查用的是漏算的估算。

## 变更
- **`crates/core/src/message.rs`**：新增 `Message::estimate_chars()` 方法，遍历**所有** `ContentBlock` 变体（Text + Reasoning + ToolUse input JSON + ToolResult content），返回忠实文本渲染供 token 估算使用。
- **`crates/llm/src/tokens.rs`**：`estimate_messages()` 从 `estimate(&m.text())` 改为 `estimate(&m.estimate_chars())`，确保估算覆盖全部会被发送给模型的内容。
- 新测试 `estimate_messages_counts_tool_results_and_tool_use`（`tokens.rs`）—— 带 ToolResult + ToolUse 的消息必须被计入。
- 更新已有测试期望值（`estimate_messages_grows_with_count` / `estimate_transcript_combines_system_and_messages`）以反映 `\n` 分隔符的加入。

## Gate
| 项 | 变更前 | 变更后 |
|----|--------|--------|
| clippy | clean | clean |
| test | 260 passed | 261 passed |
