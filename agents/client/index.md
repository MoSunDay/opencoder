Commit: (working-tree, pre-initial-commit)

# client 模块

## 职责
远端 `opencoder server` 的瘦 HTTP/SSE 客户端。镜像 server 的 JSON API 并把其 SSE `/events` 流解码回结构化事件。客户端**不持有任何本地数据、不直接调用 LLM**——每个请求都携带 bearer token 转发给 server。由 CLI 子命令 `opencode client --remote <url> --token <t>` 驱动（headless、无状态）。

## 边界与非目标
- 非目标：持久化 / LLM 调用 / 工具执行——全部发生在 server 侧。
- 非目标：自动生成 token（与 server 不同）：随机 token 无法通过鉴权，token 必须由 `--token` 或 `OPENCODER_SERVER_TOKEN` 显式提供。
- 本地/回环连接始终绕过代理（`build_http_client_with_read_timeout` 的 loopback 旁路），避免代理环境变量破坏本地连接。

## 关键抽象
- `Remote`（`src/remote.rs`）：唯一公开客户端句柄，`{ base_url, token, http: reqwest::Client }`。每个方法对请求附加 `.bearer_auth(&token)` 并经 `ensure_ok` 校验状态码。方法逐一镜像 server 路由：`health` / `list_sessions` / `create_session` / `get_messages` / `last_event_seq` / `post_prompt`（返回 `admitted_seq`）/ `switch_agent` / `switch_model` / `interrupt` / `events`。
- `events(id, after)`（`src/remote.rs`）：订阅 `GET /api/sessions/:id/events?after=N`，返回 `mpsc::Receiver<SseEvt>`（容量 128）；后台任务以 `bytes_stream` 读响应、喂 `SseFrameDecoder`、重建 `SseEvt { kind, data, ts }`。订阅端 drop（`tx.send` 失败）即停止。
- `SseFrameDecoder` / `SseFrame`（`src/sse.rs`）：增量 SSE 解码器，UTF-8 边界安全（保留不完整多字节尾）、`\r\n`/`\r` 归一化；**额外捕获 `event:` 字段**（LLM 客户端解码器只取 `data:`），使远端客户端能从 server 的 wire 格式重建细粒度 `SessionEvent` 变体。

## 主流程
`opencode client`（`crates/cli/src/client.rs::client_run`）：`resolve_token`（`--token` > `OPENCODER_SERVER_TOKEN`）→ `Remote::new` → 解析 session（显式 `--session` > `--continue` 取最近 > 新建）→ `last_event_seq` 快照游标（只流本次 prompt 产生的事件）→ `post_prompt` → `events` 循环，经 `SessionEvent::from_sse` 解码、`print_event` 回显，遇 `Done` 退出；`transcript_reset` 触发 `get_messages` 刷新（压缩重建路径）。

## 依赖与接口
- 依赖：`opencoder-core`（`Message` / `SseEvt` / `now_ms` / `build_http_client_with_read_timeout`）、reqwest、tokio、futures、serde / serde_json、anyhow、tracing。
- 被依赖：cli（`client` 子命令的运行时）；web（仅 dev-dep，用于 `tests/client_e2e.rs` 跨进程验证）。
- 运行期与 web 互不依赖；二者仅在其 e2e 测试中相遇。`Remote` 的方法集与 web 路由一一对应。

## 相关模块
- [agents/web](../web/index.md) — server 侧：`SseEvt::from_session_event` 编码（`sse_kind()`）是两 crate 共享的 wire 契约。
- [agents/cli](../cli/index.md) — `opencode client` 子命令。

## 代表性锚点
- SSE 解码单元测试：`src/sse.rs` 内联 `#[cfg(test)]`（`event:`+`data:` 解析、多 `data:` 行拼接、部分帧缓存、CRLF 归一化、`id:`/`retry:`/注释忽略、未终止帧 flush、空帧丢弃）。
- 跨进程客户端-端到端：`crates/web/tests/client_e2e.rs`（启动真实 web router + 注入 `MockChatClient`，用真实 `opencoder_client::Remote` 经 HTTP+SSE 驱动，断言回显事件序列 = server 持久化事件；并验证鉴权：正确 token 通过、错误 token → 401）。
