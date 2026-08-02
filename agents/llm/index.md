Commit: (working-tree)

# llm 模块

## 职责
OpenAI 兼容流式客户端 + LLM 抽象口子 + token 估算器。

## 关键抽象
- `ChatStream` trait（`src/stream.rs`）：`fn chat_stream(&self, req) -> Result<Receiver<LlmEvent>>`。`ChatClient`（真，reqwest+SSE）与 `MockChatClient`（测试，FIFO 脚本回放 + 请求录制）共同实现。这是 session 运行时可零 token 测试的接缝。
- `ChatClient`（`src/client.rs`）：POST `/chat/completions`，SSE 解码（`src/sse.rs`），工具调用累积（`src/tool_call.rs::ToolAccumulator`）。消息 lowering（`src/message.rs`）把 core Message 转 OpenAI JSON。`ChatParams { temperature, max_tokens }` 传递生成参数。流处理在 `tokio::spawn` 任务中跑，事件经 `mpsc::channel(128)` 投递；reqwest Client 超时 1800s / connect 30s。流错误时发 `LlmEvent::Error`（而非直接返回 Err——rx 端仍能收已缓冲事件）。重试分**两层独立预算**（策略集中在 `src/retry.rs`）：①**连接前** `MAX_ATTEMPTS=5`（`connect_with_retry`，连接/HTTP 状态码失败），②**连接后中途** `MAX_STREAM_ATTEMPTS=3`（`run_stream` 包裹 `run_stream_once`，覆盖 chunk 读取错误 / 截断流（无 `finish_reason`）/ idle 停滞三类中断）。中途重试丢弃全部累积状态（`text_buf`/`tools`/`usage`/`finished`/`decoder`）从头生成、emit `Retrying` 后重连——持久化文本永远来自单个 `Completed` 帧，绝不拼接。预算耗尽：chunk/idle → `Error`；截断 → 尽力而为 `Completed`（保留部分文本）。`idle_timeout` 内移入 `ChatClient`（`new_with_read_timeout(..., config.stream_idle_timeout(), ...)`），消费方不再各自守 idle。runner 收 `Retrying` 时清 delta buffer 并发 `Status` 事件显示 `↻ retry N/M` 徽标。
- `SseDecoder`（`src/sse.rs`）：字节级 SSE 解码器。`push(chunk)` 累积 + `drain()` 返回完整 data 帧。关键：`from_utf8` 失败时保留尾部不完整多字节序列（char 跨 TCP 读分裂），仅处理合法前缀——避免丢帧。`data: [DONE]` 终止信号正确识别。
- 重试策略（`src/retry.rs`）：纯函数模块，无 I/O。`MAX_ATTEMPTS` / `MAX_STREAM_ATTEMPTS` 两个独立预算；`is_retryable_status`（408/425/429/500/502/503/504 白名单）；`AttemptOutcome` → `RetryDecision` 的纯判定 `retry_decision`；`backoff_millis`（500ms × 2^(n-1) + 抖动）；`StreamInterruption { ChunkError, Truncated, IdleTimeout }` + `should_retry_stream_interruption`（三者均重试）。把最易出 off-by-one 的边界逻辑抽出来穷尽单测。
- `ToolAccumulator`（`src/tool_call.rs`）：`BTreeMap<usize, PartialTool>` 按工具调用 index 累积分片 `arguments` 字符串。`apply(index, delta)` 追加分片；`complete(index)` 解析 JSON `input` 得 `CompletedToolCall { id, name, input }`。`PartialTool { id, name, arguments, started }` 跟踪是否已发 `ToolCallStart`。`CompletedToolCall` 经 `pub use` 导出。
- `LlmEvent`（`src/event.rs`）：`TextDelta(String)` / `ReasoningDelta(String)` / `ToolCallStart{id,index,name}` / `ToolCallDelta{index,arguments}` / `Completed{text,tool_calls,usage}` / `Retrying{attempt,max}` / `Error(String)`。`Usage { input_tokens, output_tokens, total_tokens }` 从 `usage` JSON 提取。`ReasoningDelta` 由 `delta.reasoning_content` 字段触发（DeepSeek-V4 / glm5.2 交错思考）。
- `ChatRequest`/`ChatRequest::to_body`（`src/request.rs`）：请求体序列化。`reasoning_effort: Option<String>`（`low|medium|high`）非空时作顶层 `reasoning_effort` 字段发出，`None`/空白省略——OpenAI 风格思考深度，由 Config 透传，runner 主调用读取、compaction/title 后台调用显式置 `None`。`cache_salt: Option<String>` 非空时作顶层 `cache_salt` 字段发出，trim/空省略语义同 `reasoning_effort`——供 vLLM / prefix-cache 后端按 agent 命名空间化 KV 缓存，使会话内缓存前缀逐轮增长。
- `lower_messages`（`src/message.rs`）：把 core `Vec<Message>` 转 OpenAI `Vec<OpenAIMessage>`。关键：`ContentBlock::Reasoning` 在工具调用 turn 后序列化为 `reasoning_content` 字段回传（交错思考契约——DeepSeek-V4 缺此字段 HTTP 400）。 **多模态/工具图片契约**：`tool_message()` 永远输出字符串 content（OpenAI 规定 `role:"tool"` 必须是纯字符串）；工具返回的图片经 `tool_image_user_message()` 重安置到一条尾随的 `role:"user"` 多模态消息（合法的 `image_url` 块），`push_tool_results()` 与 `push_user()` 均在检测到图片时 emit 该重安置消息——修复了把图片塞进 tool 消息 content 的 spec 违规（旧实现被 provider 拒绝/静默丢图）。
- `tokens::{estimate, estimate_messages, estimate_transcript}`（`src/tokens.rs`）：chars/4 启发式，供压缩首轮触发判断。`estimate_messages` 调用 `Message::estimate_chars()`（覆盖 ToolResult/ToolUse input/Reasoning，而非仅 `Message::text()` 的 Text 块）——这是压缩首轮能真正触发的关键。
- `MockChatClient`（`src/mock.rs`）：`scripts: Vec<Vec<LlmEvent>>` FIFO 回放——每调 `chat_stream` 弹一个脚本。`requests()` 返回已录制的 `Vec<ChatRequest>`，供集成测试断言「模型实际收到什么」（如 plan_handoff 集成测试验证 act 请求结构）。这是 session/tui 集成测试的零 token 接缝。

## 依赖与接口
- 依赖：reqwest（rustls）、tokio、opencoder-core（Message）。
- 被依赖：session（runner/compaction/title）、web（POST /prompt 建客户端）、cli/tui。

## 相关模块
- [agents/session](../session/index.md) — 经 ChatStream 驱动 agent loop。
- [agents/core](../core/index.md) — Message 类型来源。
