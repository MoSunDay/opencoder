# opencode server / client：中心化存储 + 远程 headless 客户端

## 背景

此前 `opencode serve` 只是把本地 TUI 背后的 HTTP/SSE 服务暴露出来，存储与 LLM 调用都落在运行 TUI 的那台机器上。本次新增「中心化部署」形态：一台机器跑 `opencode server`（持有 SQLite 存储 + 作为 LLM 网关），其它机器用瘦客户端 `opencode client` 经 SSE 把 prompt 投递到远端、并实时拉回事件流。这样多机协作、CI/容器化、远程跑批时数据集中、密钥集中。

两个子命令都需要 bearer-token 鉴权，防止端口被扫后任何人都能白嫖模型或读会话历史。token 解析顺序：`--token <T>` > `OPENCODER_SERVER_TOKEN` 环境变量 >（仅 server）自动生成 ULID 打到 stderr。

## 变更

### 重命名 `Command::Serve` → `Command::Server`（`crates/cli/src/lib.rs`）

`Serve` 子命令改名为 `Server`，并保留 `serve` 作为别名以兼容旧脚本。`--token` 参数下沉到 server 子命令。删除了原先无用的 `Cli.serve` 字段与 `serve_launch`。`src/main.rs` 派发改为 `Command::Server` / `Command::Client`。

### 新增 `crates/client/`（瘦远程客户端）

- `remote.rs`：`Remote` 结构体封装所有远端调用——`health` / `list_sessions` / `create_session` / `get_messages` / `last_event_seq` / `post_prompt` / `switch_agent` / `switch_model` / `interrupt` / `events`（SSE）。所有写操作附带 `Authorization: Bearer <token>`；SSE 读取额外支持 `?token=<T>` 查询参数（EventSource 无法设 header）。
- `sse.rs`：`SseFrameDecoder`——增量式 SSE 帧解析器，跨缓冲区拼接 `data:` 行、处理 CRLF、忽略 `id:`/`retry:`/注释、空白 `data` 帧丢弃。

### 共享 `SseEvt` 上移到 core（`crates/core/src/sse.rs`）

原先 web 内部专用的 `SseEvt` 结构体移到 `opencoder-core`，server 与 client 共用。web 重新导出（`crates/web/src/handle.rs`）。

### `SessionEvent::from_sse()` 逆映射（`crates/session/src/runner.rs`）

`from_sse(kind, data)` 是 `sse_kind()` + `sse_data()` 的逆运算，覆盖全部 16 个事件变体。客户端拿到 server SSE 帧后用它还原成强类型 `SessionEvent` 再走 `print_event` 渲染。注意 `TranscriptReset` 在线上是 lossy 的（重建后 messages 为空）。

### `last_event_seq` 进入 Store trait（`crates/store/src/store.rs`）

新增 trait 方法 `last_event_seq(session_id) -> i64`，libsql 实现落在 `events.rs::last_seq`。客户端在投递 prompt 前先快照当前 max seq，以便只渲染「新事件」。server 侧 `crates/web/src/api.rs` 新增 `get_event_seq` handler 暴露该值；prompt 投递返回体字段名为 `admitted_seq`（不是 `seq`），客户端读取时带 fallback。

### Bearer-token 鉴权中间件（`crates/web/src/auth.rs`）

`require_token` 中间件同时接受 `Authorization: Bearer <T>` 头与 `?token=<T>` 查询参数，二者任一匹配即放行，否则 401。经 `from_fn_with_state` 挂到 `build_app()` 上。`manager.html` 的 JS 从 URL 取 `?token=` 存 localStorage，并自动附到所有 fetch / EventSource 请求。

### `serve()` 签名重构 + 确定性 e2e 测试钩子（`crates/web/src/lib.rs`）

`build_app(state, token)` 抽成 pub fn 供测试直接构造 router；`serve()` 改为接收 `token: String` 参数；`AppState` 新增 `client_override: Option<Arc<dyn ChatStream>>`，使 e2e 测试可注入 `MockChatClient` 做零 token 确定性验证，不走真 LLM。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| `from_sse` 16 变体完整往返 | `from_sse_roundtrips_all_variants` | `crates/session/src/runner.rs` |
| `from_sse` 未知 kind 返回 None | `from_sse_unknown_kind_is_none` | `crates/session/src/runner.rs` |
| `from_sse` 缺字段返回 None | `from_sse_missing_field_is_none` | `crates/session/src/runner.rs` |
| 查询 token 解析成对 | `query_token_finds_pair` | `crates/web/src/auth.rs` |
| 查询 token 缺失 | `query_token_absent` | `crates/web/src/auth.rs` |
| 无 token 访问 → 401 | `health_without_token_is_401` | `crates/web/tests/auth.rs` |
| 错误 bearer → 401 | `health_with_wrong_bearer_is_401` | `crates/web/tests/auth.rs` |
| 正确 bearer → 200 | `health_with_correct_bearer_is_200` | `crates/web/tests/auth.rs` |
| 正确查询 token → 200 | `health_with_correct_query_token_is_200` | `crates/web/tests/auth.rs` |
| manager.html 受 token 保护 | `index_html_protected_by_query_token` | `crates/web/tests/auth.rs` |
| sessions 列表需 token | `sessions_list_requires_token` | `crates/web/tests/auth.rs` |
| health 正确 token 通过 / 错误失败 | `health_with_correct_token_succeeds_wrong_token_fails` | `crates/web/tests/client_e2e.rs` |
| 客户端 echo 与 server 持久化事件一致 | `client_echo_matches_server_persisted_events` | `crates/web/tests/client_e2e.rs` |
| get_messages 返回 transcript | `client_get_messages_returns_transcript` | `crates/web/tests/client_e2e.rs` |
| list_sessions 返回已建会话 | `list_sessions_returns_created_session` | `crates/web/tests/client_e2e.rs` |
| SSE 单事件 + data | `parses_event_and_data` | `crates/client/src/sse.rs` |
| SSE 多 data 行拼接 | `joins_multiple_data_lines` | `crates/client/src/sse.rs` |
| SSE 跨缓冲区保留 partial | `holds_partial_until_blank_line` | `crates/client/src/sse.rs` |
| SSE 规整 CRLF | `normalizes_crlf` | `crates/client/src/sse.rs` |
| SSE 忽略 id/retry/注释 | `ignores_id_and_retry_and_comments` | `crates/client/src/sse.rs` |
| SSE flush 剩余未终止帧 | `flush_remaining_emits_unterminated_frame` | `crates/client/src/sse.rs` |
| SSE 空 data 帧丢弃 | `empty_data_frame_is_dropped` | `crates/client/src/sse.rs` |
| server 子命令 + serve 别名 | `server_subcommand_and_serve_alias` | `crates/cli/tests/cli_parse.rs` |
| client 子命令解析 | `client_subcommand_parses` | `crates/cli/tests/cli_parse.rs` |
| server token 解析优先级 | `resolve_token_param_wins` | `crates/cli/src/server.rs` |
| client token 解析优先级 | `resolve_token_param_returns_ok` | `crates/cli/src/client.rs` |

此外做了一次真 TCP 端到端冒烟（`opencode server --port 18234 --token smoke123` + `opencode client --remote http://127.0.0.1:18234 --token smoke123 hello echo`）：客户端拿到 MockChatClient 流式回复并打印 `[remote session <ULID>]`；错误 token 的客户端被干净地 401 拒绝、退出码 1。

## Gate

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 615 passed / 0 failed |
| `cargo clippy`（core/store/llm/session/web/client/cli + bin opencoder，`--all-targets -D warnings`） | 零警告 |
| `cargo build --workspace` | 零错误 |
| 实跑冒烟 `opencode server` + `opencode client` | 通过 |
