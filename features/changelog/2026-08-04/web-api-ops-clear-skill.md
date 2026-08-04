# feat(web,store): Web API 运维操作端点 + DrainCmd 通道 + clear_skill

## 背景

此前 HTTP server 仅暴露 session prompt/SSE 的核心对话能力，缺少运维级
操作（fork 会话、手动压缩、plan→act handoff、config 热重载、live skill
切换）。这些操作需要 `&mut SessionState`，而该可变状态仅存在于后台 drain
任务内，无法从请求处理路径直接触达。同时 store 层的 `SessionPatch` 只有
"跳过" 语义（`skill: None` = 不动），无法表达 "把 skill 字段清成 NULL"。

本次新增 8 个 HTTP 端点、一个 `DrainCmd` 命令通道把请求转发到 drain 任务，
并在 store 层补充 `clear_skill` 显式清除能力，使 web 层与 CLI/TUI 的运维
能力对齐。

## 变更

### DrainCmd 命令通道 — `crates/web/src/cmd.rs`（新增）

`DrainCmd` 枚举，4 个变体：
- `Compact` — 触发手动压缩。
- `Handoff { extra }` — plan→act handoff，携带额外指令文本。
- `SetSkill(Option<String>)` — live skill 切换；`None` = 清除当前 skill。
- `ReloadConfig` — 热重载 config。

### SessionHandle 布线 — `crates/web/src/handle.rs`

- `SessionHandle` 持有 `cmd_tx: UnboundedSender<DrainCmd>` 与
  `cmd_rx: Mutex<Option<UnboundedReceiver<DrainCmd>>>`。
- `send_cmd(handles, session_id, cmd) -> bool`：把命令投递到指定 session 的
  通道，找不到返回 `false`。
- `process_cmd(state, cmd)`：drain 任务在每次 `run` 完成后消费接收端，对
  `&mut SessionState` 有序应用命令（Compact / Handoff / SetSkill）。

### 8 个新 HTTP 端点 — `crates/web/src/api_ops.rs`（新增）

| 方法 | 路径 | handler |
|------|------|---------|
| POST | `/api/sessions/:id/fork` | `fork_session` |
| POST | `/api/sessions/:id/compact` | `post_compact` |
| POST | `/api/sessions/:id/handoff` | `post_handoff` |
| POST | `/api/sessions/:id/skill` | `post_skill` |
| GET  | `/api/config` | `get_config` |
| PATCH| `/api/config` | `patch_config` |
| GET  | `/api/bg` | `list_bg` |
| POST | `/api/bg/stop` | `stop_bg` |

路由注册于 `crates/web/src/lib.rs`。fork/compact/handoff/skill 通过
`send_cmd` 投递 `DrainCmd`；config 读写直接走 `Config::load` /
`Config::merge`。辅助函数 `load_config` / `build_client` 返回
`Result<T, Box<Response>>`（`Box` 把 Err 变体收敛为指针大小，避免
`result_large_err`）。

### clear_skill store 能力 — `crates/store`

- `types.rs`：`SessionPatch` 新增 `clear_skill: bool`（`#[serde(default,
  skip_serializing_if = "is_false")]`，向后兼容）。
- `libsql_store/sessions.rs`：`update` 中 `if patch.clear_skill { sets
  .push("skill = NULL") }`。

## 兼容性

- `SessionPatch::clear_skill` 默认 `false`，`#[serde(default)]` 使旧客户端
  不发送该字段时行为不变。`skill: None` 仍表示 "跳过"，二者正交。
- `DrainCmd` 通道为新增布线，不改动既有 drain 主循环的 steer/queue 提升
  语义；无命令时接收端为空，零开销。

## 测试清单

- `cargo build --workspace` — 通过（0 warning）
- `cargo clippy --workspace --all-targets -- -D warnings` — 通过（0 warning）
- `cargo test -p opencoder-web --test web_api_ops` — 12 passed; 0 failed
- `cargo test -p opencoder-store --test clear_skill` — 2 passed; 0 failed
- `cargo test -p opencoder-store` — 66 passed; 0 failed
- `cargo test --workspace` — 全量回归通过（1799 passed; 0 failed; 1 ignored）

### web_api_ops 覆盖（12）

fork（拷贝消息返回新 id / 不存在返回 404 / title 加 fork 后缀）、skill
（持久化到 store meta / `null` 清除）、config（GET 返回 JSON / PATCH 合并
并持久化）、bg（list 空数组 / stop 返回 ok）、compact（ok 并入队 /
不存在 404）、handoff（存在 plan 时 ok）。

### clear_skill 覆盖（2，新增）

`clear_skill_nulls_skill_field`（true 后 skill 被 NULL 化、无关字段不动）、
`default_patch_leaves_skill_intact`（默认 patch 不触碰 skill 字段）。
