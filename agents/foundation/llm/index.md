Commit: 3cc5011a805b0b3f8110623a5c4f0be40267285c

# llm 模块

OpenAI 兼容流式模型客户端。细节以代码为准。
接缝：`Arc<dyn ChatStream>`，session/tui/web 经此消费模型。

## 索引
- [src/client.rs](../../../crates/llm/src/client.rs) — OpenAI 兼容流式请求
- [src/stream.rs](../../../crates/llm/src/stream.rs) — `ChatStream` trait
- [src/mock.rs](../../../crates/llm/src/mock.rs) — `MockChatClient`（测试）
- [src/tokens.rs](../../../crates/llm/src/tokens.rs) — token 估算
- [src/responses/](../../../crates/llm/src/responses/mod.rs) — Responses 协议请求与解码
- [src/retry.rs](../../../crates/llm/src/retry.rs) — 流式重试策略（纯函数）
- [src/embed.rs](../../../crates/llm/src/embed.rs) — `/embeddings` 与 `ChatStream::embed`
