Commit: (working-tree, pre-initial-commit)

# llm 模块

## 职责
OpenAI 兼容流式客户端 + LLM 抽象口子 + token 估算器。

## 关键抽象
- `ChatStream` trait（`src/stream.rs`）：`fn chat_stream(&self, req) -> Result<Receiver<LlmEvent>>`。`ChatClient`（真，reqwest+SSE）与 `MockChatClient`（测试，FIFO 脚本回放 + 请求录制）共同实现。这是 session 运行时可零 token 测试的接缝。
- `ChatClient`（`src/client.rs`）：POST `/chat/completions`，SSE 解码（`src/sse.rs`），工具调用累积（`src/tool_call.rs::ToolAccumulator`）。消息 lowering（`src/message.rs`）把 core Message 转 OpenAI JSON。
- `ChatRequest`/`ChatRequest::to_body`（`src/request.rs`）：请求体序列化。`reasoning_effort: Option<String>`（`low|medium|high`）非空时作顶层 `reasoning_effort` 字段发出，`None`/空白省略——OpenAI 风格思考深度，由 Config 透传，runner 主调用读取、compaction/title 后台调用显式置 `None`。
- `tokens::{estimate, estimate_messages, estimate_transcript}`（`src/tokens.rs`）：chars/4 启发式，供压缩首轮触发判断。`estimate_messages` 调用 `Message::estimate_chars()`（覆盖 ToolResult/ToolUse input/Reasoning，而非仅 `Message::text()` 的 Text 块）——这是压缩首轮能真正触发的关键。

## 依赖与接口
- 依赖：reqwest（rustls）、tokio、opencode-core（Message）。
- 被依赖：session（runner/compaction/title）、web（POST /prompt 建客户端）、cli/tui。

## 相关模块
- [agents/session](../session/index.md) — 经 ChatStream 驱动 agent loop。
- [agents/core](../core/index.md) — Message 类型来源。
