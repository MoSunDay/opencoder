Commit: a8a742ba7c4b952173e1403d3e11da809d3f80cf

# llm 模块

OpenAI 兼容流式模型客户端。细节以代码为准。
接缝：`Arc<dyn ChatStream>`，session/tui/web 经此消费模型。

## 索引
- [src/client.rs](../../crates/llm/src/client.rs) — OpenAI 兼容流式请求
- [src/stream.rs](../../crates/llm/src/stream.rs) — `ChatStream` trait
- [src/mock.rs](../../crates/llm/src/mock.rs) — `MockChatClient`（测试）
- [src/tokens.rs](../../crates/llm/src/tokens.rs) — token 估算
