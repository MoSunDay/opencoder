Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# llm 模块

OpenAI 兼容流式客户端 + ChatStream 抽象 + token 估算。

## 关键路径
- `src/stream.rs` — `ChatStream` trait：`chat_stream -> Receiver<LlmEvent>`。
- `src/client.rs` — POST /chat/completions；read idle 默认 600s、connect 30s。
- `src/retry.rs` — 双层预算 `MAX_ATTEMPTS=5` / `MAX_STREAM_ATTEMPTS=3`。
- `src/retry.rs::retry_delay` — Retry-After floor 1s、cap `RETRY_AFTER_MAX_SECS=120`。
- `src/retry.rs::is_retryable_status` — 408/425/429/500/502/503/504 白名单。
- `src/sse.rs` — `SseDecoder` 字节级解码，保留跨 chunk 多字节尾部。
- `src/tool_call.rs` — `ToolAccumulator` 按 index 累积工具调用分片。
- `src/event.rs` — `LlmEvent`：TextDelta/ReasoningDelta/ToolCall*/Completed/Retrying/Error。
- `src/client.rs::extract_reasoning` — reasoning_content 等别名字段 + 末帧 message 回退。
- `src/request.rs` — `reasoning_effort`/`cache_salt` 非空才发顶层字段。
- `src/message.rs::lower_messages` — Reasoning 回传 `reasoning_content`；工具图片重安置 user 消息。
- `src/tokens.rs` — chars/4；`estimate_messages` 含 +4 overhead，`*_for_display` 不含。
- `src/embed.rs` — POST /embeddings；`EMBED_MAX_ATTEMPTS=3`、每请求 60s 超时。
- `src/embed.rs::embeddings_via` — 同步桥（block_in_place / 独立 runtime）。
- `src/mock.rs` — `MockChatClient` FIFO 脚本回放 + 请求录制，零 token 接缝。

## 边界
- 中途重试丢弃全部累积状态从头生成；持久化文本只来自单个 Completed 帧。
- 流错误经 `LlmEvent::Error` 投递，不直接返回 Err。

## 相关
- [agents/session](../session/index.md) — 经 ChatStream 驱动 agent loop。
- [agents/core](../core/index.md) — Message 类型来源。
