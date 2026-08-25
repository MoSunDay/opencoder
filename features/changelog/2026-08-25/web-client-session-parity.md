Commit: (working-tree)

# web/client 会话能力对齐：subagent 列表、clear-all、子会话视图、bg 面板

## 背景

`crates/web`（server API + 内嵌 SPA）与 `crates/client`/`crates/cli`（`opencode client`）相对 TUI 存在 4 个未记录的会话能力差距：子代理任务无只读列表端点、无 clear-all、SPA 子代理卡片刷新即丢且无法查看子会话 transcript、后台进程无 UI。本轮全部补齐，**严格零改动 `crates/tui` / `crates/session` / `crates/store`**（所需 `Store` trait 方法均已存在）。

## 变更

### Server（crates/web）

- 新模块 `src/api_subagents.rs`（约 190 行）：
  - `GET /api/sessions/:id/subagents` → `{tasks:[{id,kind,status,child_session_id,prompt,parent_message_id,result,ok,created_at,updated_at}]}`（`kind` 映射存储 `agent` 字段；`created_at`/`updated_at` 镜像 `started_at`/`completed_at`，in-flight 时回落 `started_at`）。会话不存在 404；存在但空列表 200。
  - `DELETE /api/sessions?keep=:id`（clear-all）：任一 live handle draining → 409（照抄 TUI `gate_clear_all` 飞行中拒绝语义，keep 自身 draining 同样拒绝）；否则对非 keep 的 live handle 复用 `delete_session` 的 evict 语义（cancel drain + fire child cancels + abandon 等待中 question + MCP cleanup），再 `store.clear_other_sessions(keep)` FK 级联。缺 `keep` 参数 400、keep 不存在 404、幂等重放 `removed:0`。
  - 路由在 `lib.rs::build_app()` 注册（collection 路由追加 `.delete`，与既有 `:id` DELETE 无冲突）。
- SPA（classic 全局脚本，不改 chat.js）：
  - `assets/subagent_view.js`（新，约 200 行）：`subagent_start` SSE 为 live 卡片补挂 expand 按钮（带 `child_session_id`）；wrap `loadTranscript` 在快照渲染后按 `/subagents` 列表**恢复历史子代理卡片**（解决刷新丢失；终态卡片去 steer 按钮、补结果 tail）；点击经 **#log 事件委托**打开抽屉，拉 `GET /api/sessions/:child_id/messages`（既有端点，不过滤 task_type）渲染子会话 transcript（复用 chat.js `mkMsgDiv`）。
  - `assets/bg_panel.js`（新，约 65 行）：settings 弹窗内 bg 进程列表（`GET /api/bg`）+ stop-all（`POST /api/bg/stop` → sys-chip 回显 killed 数）；开面板/回前台 + 5s 轮询。
  - `index.html` 占位符（`bg-list` 行 + `JS_SUBAGENTS`/`JS_BG` markers）、`styles.css` 抽屉/面板样式、`html.rs` const/scripts 数组/skeleton+sentinel 测试同步。

### Client / CLI

- `crates/client/src/remote_ops.rs`：新增 `list_subagents(id)`（→ `Vec<Value>`）与 `clear_sessions(keep)`（→ removed 数），照抄 `ensure_ok` + `serde_json::Value` 进出模式。
- `crates/cli`：`ClientSessionSub::{Tasks{id}, Clear{keep}}`；`client_ops.rs` 分发（pretty JSON / `cleared N session(s)`）；`cli_parse.rs` 解析断言。

## 设计说明

- `child_session_id` 以 **list 端点为权威数据源**（持久），SSE `subagent_start` 仅作 live 加速（两路都可用，UI 先读 dataset、miss 时回落 list）。
- clear-all 的 check-then-act 窗口与 TUI gate 相同（窗口内新起的 drain 写已删会话会显式报错）；`any_draining` 辅助在 map 锁下取一致快照。
- 未扩权：文档化排除项继续排除（notepad、copy mode、keymap、`!cmd`、TODO 工作流、EditPlan、CORS）；`ts_origin`/`ts_mirror` 属 TUI 内部观测机制不纳入。

## 测试清单（rules/01、02、03）

- 集成（web）：`tests/subagent_list_api.rs`（新，2）：字段形状/kind 映射/created_at-updated_at 语义、空列表 200 vs 缺失 404；`tests/clear_sessions_api.rs`（新，4）：keep 保留 + 子代理子会话/任务行级联、draining 409（任意/keep 自身）、非 keep handle evict 而 keep handle 保留、缺参 400/缺会话 404/幂等。
- 集成（client↔server）：`tests/client_remote_ops.rs` 扩 1：`list_subagents_and_clear_sessions_roundtrip`（404/空列表/字段、removed=2 级联、幂等 0、clear 404）。
- e2e（node smoke，缺 node 自动 skip）：`frontend_smoke.mjs` S7（14 断言）：卡片恢复、终态/运行中卡片形状、expand → 子会话抽屉 → 关闭、bg 面板列表/stop-all/清空；shim 增强（listener 记录、`document.getElementById`、per-session messages）。
- 单测：`src/html.rs` skeleton（`bg-list`）+ sentinel 依赖序（subagent_view→bg_panel）；`crates/cli/tests/cli_parse.rs`（新 2）：`session tasks`/`session clear <keep>` 解析。
- 手验：真实 `opencoder serve` + curl（空列表 200/404/400/405→200 语义、removed 计数、幂等、SPA 内联脚本）+ `opencode client session tasks/clear` 全链路。
- 回归：`cargo test --workspace` 220 个测试目标 3298 passed / 0 failed（提交前新鲜重跑）；`git diff` 确认本次会话对 `crates/tui`/`crates/session`/`crates/store` 零改动（工作区中预存的 tui 未提交改动系前一轮遗留，未触碰）。

## 兼容性

- 纯新增端点/方法/资产，无 schema、既有 API、配置变化；`DELETE /api/sessions/:id`（单个）语义不变。
