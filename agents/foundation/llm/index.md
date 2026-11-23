Commit: 0abfe0c77

# llm 模块

OpenAI 兼容流式模型客户端。细节以代码为准。
接缝：`Arc<dyn ChatStream>`，session/tui/web 经此消费模型。

## 索引
- [src/client.rs](../../../crates/llm/src/client.rs) — OpenAI 兼容流式请求
- [src/stream.rs](../../../crates/llm/src/stream.rs) — `ChatStream` trait
- [src/sse.rs](../../../crates/llm/src/sse.rs) — SSE 解码器：pending 缓冲 64MiB 硬上限（无终止符字节流超限即 `SseOverflow` 终止流，chat 与 responses 两条流路均生效），防上游灌爆内存
- [src/mock.rs](../../../crates/llm/src/mock.rs) — `MockChatClient`（测试）
- [src/tokens.rs](../../../crates/llm/src/tokens.rs) — token 估算
- [src/responses/](../../../crates/llm/src/responses/mod.rs) — Responses 协议请求与解码
- [src/retry.rs](../../../crates/llm/src/retry.rs) — 流式重试策略（纯函数）：pre-stream 10 次预算（0.5 s 起指数退避，单次等待 ≤5 min），中流 3 次；`Retry-After` ≤300 s 并与退避取 max
- [src/sanitize.rs](../../../crates/llm/src/sanitize.rs) — 请求体工具配对兜底（crate 私有纯函数）
- [src/embed.rs](../../../crates/llm/src/embed.rs) — `/embeddings` 与 `ChatStream::embed`
