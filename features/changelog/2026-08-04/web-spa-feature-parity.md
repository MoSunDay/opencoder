Commit: (working-tree, pre-initial-commit)

# feat(web): SPA 功能对齐 TUI — drain 命令通道 + 8 个 API 端点 + 全新 SPA

## 背景

Web 前端（HTTP API + 单页应用）与 TUI 存在显著能力差距：后端缺少 compact /
handoff / config / skill / fork 等 `&mut SessionState` 操作端点；旧 `manager.html`
仅有 97 行、仅处理 `done`/`error` 两种事件，存在 forEach bug、SSE onmessage 为空、
无 interrupt / 删除 / 模型切换 / 图片上传 / steer 等。

本轮分三阶段补齐至功能对齐。

基线：workspace 0 failed。本轮后：1799 passed / 0 failed / 1 ignored（pre-existing）。

## 变更

### Phase 0 — Drain 命令通道（session→web）
**文件**: `crates/web/src/cmd.rs`(新), `crates/web/src/handle.rs`

`SessionState` 仅存活于后台 `drain_to_completion` 任务内。需要 `&mut SessionState`
的操作（compact / handoff / skill / config reload）通过 mpsc 命令通道排队，
在 `run()` 完成后于 drain 闭包内串行处理。

- `DrainCmd` 枚举：`Compact`、`Handoff{extra}`、`SetSkill(Option<String>)`、`ReloadConfig`
- `SessionHandle` 新增 `cmd_tx`/`cmd_rx`（`Arc<std::sync::Mutex<CmdRx>>`，持锁不跨 await）
- `CmdRxGuard`：drop 时归还 `cmd_rx`（panic 安全）；`DrainGuard`：drop 时清 `draining`
- `ensure_drain()` / `send_cmd()` / `apply_drain_cmd()` / `process_drain_cmds()`

### Phase 1 — 8 个新 API 端点
**文件**: `crates/web/src/api_ops.rs`(新), `crates/web/src/lib.rs`, `crates/web/src/api.rs`

| 端点 | 方法 | 说明 |
|---|---|---|
| `/api/sessions/:id/fork` | POST | fork 会话（web 内联实现，避免 web→cli 循环依赖） |
| `/api/sessions/:id/compact` | POST | 触发压缩（排 DrainCmd::Compact） |
| `/api/sessions/:id/handoff` | POST | plan→act 交接 |
| `/api/sessions/:id/skill` | POST | 设置/clear skill |
| `/api/config` | GET | 读配置 |
| `/api/config` | PATCH | 热更配置（ReloadConfig） |
| `/api/bg` | GET | 后台任务列表 |
| `/api/bg/stop` | POST | 停止后台任务 |

- `PromptBody` 增加 `skill: Option<String>`，`post_prompt` 持久化 skill
- `SessionPatch` 增加 `clear_skill: bool`（区分"置 NULL"与"不修改"），libsql update 对应处理

### Phase 2 — SPA 全量重写
**文件**: `crates/web/src/assets/index.html`、`styles.css`、`render.js`、`app.js`（新）,
`crates/web/src/html.rs`（重写）；删除旧 `manager.html`

单二进制理念：所有资源 `include_str!` 内联为单一 HTML 文档（`LazyLock` 编译期拼接，
无静态文件服务）。

修复与新增能力：
- **forEach bug 修复**：会话列表正确迭代 `sessions` 数组
- **实时 SSE 增量渲染**：处理全部 17 种事件类型（text_delta / reasoning_delta /
  tool_start / tool_end / compaction / compaction_delta / status / agent_switched /
  model_switched / plan_handoff / transcript_reset / subagent_start / subagent_end /
  autopilot / steer_consumed / queue_consumed / done），逐 token 流式回显
- **interrupt**：按钮中断 + Esc 键（中断以 `status:"interrupted"` 投递）
- **删除会话 / fork / compact** 按钮
- **模型切换 / agent 切换**：修复 act/plan 语义错配
- **steer / queue 投递**：Enter=steer，Shift+Enter=queue
- **图片上传**：粘贴 / 文件选择，dataURL 回显 + 预览
- **思考块折叠**、工具结果展开、compaction 摘要、subagent 状态、autopilot 迭代
- **事件名契约对齐**：后端应用错误以 `event: error` 投递（非 `error_evt`），
  中断以 `status` 事件投递（无 `interrupted` 事件）——SPA 已对齐

## 测试覆盖

| 测试文件 | 数量 | 说明 |
|---|---|---|
| `crates/web/tests/web_api_ops.rs` | 12 | fork/skill/compact/handoff/config/bg 全端点（本轮新） |
| `crates/web/src/html.rs`(unit) | 2 | 标记替换 + 资源内联 + 脚本顺序（本轮新） |
| `crates/web/tests/web_drain_contract.rs` | 6 | drain 命令通道契约（Phase 0） |
| `crates/web/tests/web_contract.rs` | 13 | 会话/SSE/prompt 既有契约 |
| 其余 web 集成 | 21 | auth/bugfix_contracts/client_e2e/replay/subagent_steer/image |

web crate 合计：60 passed / 0 failed。
workspace：1799 passed / 0 failed / 1 ignored(pre-existing) / 0 clippy warnings。

## 约束遵循

- 新文件均 ≤400 行（render.js 196 / app.js 165 / api_ops.rs 273 / html.rs 50）
- 迭代文件均 ≤800 行（handle.rs 413 / api.rs 614）
- 纯函数式：无 class，命令通过枚举 + 函数分派，状态经参数传递
