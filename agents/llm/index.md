Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# llm 模块

OpenAI 兼容流式模型客户端。细节以代码为准。

## 索引
- `src/client.rs` — OpenAI 兼容流式请求
- `src/chat_stream.rs` — `ChatStream` trait
- `src/mock.rs` — `MockChatClient`（测试）
- `src/tokens.rs` — token 估算

## 接缝
- `Arc<dyn ChatStream>`：session/tui/web 经此消费模型。
