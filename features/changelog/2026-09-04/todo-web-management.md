Commit: (working-tree, 待提交)

# TODO 模板/Env/工具 share 树 + Web 管理面与 SPA

## Context

todos 框架（`crates/todos`）此前仅 CLI 消费：无 `/api/todos*` 端点、无 UI。本轮落地「NFS 兼容纯目录 share 树」并补齐 web 管理面：

- **M1 存储层**（core）：新增 `share_fs` 模块 + `AgentDefaults.share_dir` 配置（`OPENCODER_SHARE_DIR` → 默认 `~/.opencoder/share`，可指到 NFS 挂载点）。布局：`<share>/todo/<name>/todo.json`（元数据）、`<share>/todo/<name>/<version>/context.json`（= WorkflowSpec JSON）、`.../env.json`（env 绑定）、`<share>/env/<name>/context.json`、`<share>/agent/tools/<version>/<tool>`（CLI 工具，可从 `~/.opencoder/agents/<n>/tools/v{n}/<t>` 导入）。全部 tmp+rename 原子写，名字规则禁 `/`、`\`、`..`、`\0`（穿越安全）。NFS server 本身不实现（`AgentNfsConfig` 仍留作后续挂载暴露的开关）。
- **M2 后端**（web）：4+1 个新模块 17 条路由（HMAC 签名自动覆盖）：
  - `api_todo_envs.rs`：env CRUD（PUT 校验工具引用可解析，失败 400）+ `GET /api/todo/tools`（share 内 + agents 根可导入项）+ `POST /api/todo/tools/import`。
  - `api_todo_templates.rs` / `api_todo_template_versions.rs`：模板 CRUD、`todo.json` 元数据、`context.json` GET/PUT（PUT 走 `parse_spec`+`validate_spec`，400 带校验错误）、`env.json` 绑定（校验 env 存在）、`new-version`（复制 context/env、翻 current）、删除版本（current 409 保护）。
  - `api_todo_runs.rs`：`POST .../run` 读 context+env → 校验 → env 名与已解析工具写入 `spec.metadata.env/env_tools` → 仿 `post_prompt` 构建 ChatClient（`client_override` 优先）→ `tokio::spawn` Runtime → 返回 workflow_id；workflows 列表/详情（复用 Store 投影查询）；interrupt（复用 `opencoder_todos::interrupt`，跨进程 store CAS 已支持）；resume（拒绝 Running，409）。
  - `todo_hub.rs`：`GET /api/todo/workflows/:id/events?after=` SSE——**store 轮询**（500ms）而非进程内广播：Runtime 经共享 Store 提交事件、无进程内生产者通道，轮询同时覆盖 CLI 跨进程驱动的工作流；终帧（workflow_completed/failed）后关流。
- **M3 SPA**：菜单新增 「TODO 管理」「Env 管理」；`todoPanel`（模板表+版本管理+运行分发）、`todoEditor`（表单/JSON 双模编辑 WorkflowSpec + env 绑定）、`todoRunsPanel`（workflow 列表/条目/SSE 事件流/中断恢复）、`envsPanel`（env 编辑 + 工具目录/导入）；`api.js` 补 `apiPut`。
- 工具注入运行时（`ToolContext.tools_path` 置 Some）为后续任务，seam 已在。

## 关键决策

- share 根解析仿 `agents_dir` 三级：进程 override（测试）→ env → config → `<global home>/share`；集成测试用 `set_share_dir_override` + 每文件互斥锁串行隔离。
- `context.json` 即现有 `WorkflowSpec`，前端编辑的就是它（零新 schema）。
- 与既有 `/api/envs`（配置快照）正交，新命名空间 `/api/todo/envs`。

## Tests

- unit：`crates/core/src/share_fs_tests.rs`（6 用例：名字规则/穿越拒绝、原子写无 tmp 残留、目录列举、工具引用形状+存在性、根解析链）；`api_todo_util.rs` 内嵌版本号用例。
- integration：`crates/web/tests/web_todo_envs.rs`（6：CRUD 回环、工具引用 400→补种子→200、tools 双源枚举、import 字节复制、404、body 穿越 400）；`web_todo_templates.rs`（9：创建/列表/读取、环依赖 400、未知 agent 400、context PUT 非法 400、new-version/current 翻转、删非 current/删 current 409、env 绑定 400→200、模板删除）；`web_todo_runs.rs`（MockChatClient 经 `client_override` 驱动至终态、列表/详情、running 时 resume 409→interrupt→resume、SSE 200/content-type/终帧关流/404）。
- SPA vitest：`todoPanel.dom.test.jsx`（3）、`envsPanel.dom.test.jsx`（4）——mock api.js，覆盖渲染、运行分发 POST、新建模板 spec 携带、导入/保存 PUT 合并体。
- 回归：`cargo test --workspace --exclude opencoder-team`（team 为并行迭代 WIP，见下）；`cd crates/web/spa && npm test` 293 用例全绿；`npm run build` 重产 dist（固定名单单 bundle 不变）。

## 协作说明

本轮与 dag/server-split/team 等并行迭代共享工作树：workspace 全量回归中 `opencoder-team`（他人 WIP，编译中）与 `daemon_smoke`/`nodes_smoke_proc`/`running_mode_switch_e2e`（daemon→`opencode-server` 拆分进行中）、`opencoder-dag` 自身单测（新增中）为并行工作失败项，与本特性无关；本特性涉及 crate（core/web/todos + spa）全绿。
