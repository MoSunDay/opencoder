Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# Agent 模式「执行 Agent」下拉只列 Agent 配置注册卡

## 变更

- chat 页（nav「Agent」）Agent 模式的「执行 Agent」select，候选集从「内置
  act/plan/command 在前 + 注册卡」改为只来自 Agent 配置（`GET /api/agents`）
  的 primary 注册卡：内置三角色是 operator 宿主循环的角色，不再进入 Agent
  执行泳道的选择面。默认值链（`useState('act')`、lane 重置、打开会话回填）
  全部按模式收敛——Agent 模式默认第一张注册卡，Operator 模式维持 `act`。
- 配置为空时：下拉显示「暂无可用 Agent」占位，发送被门禁拦截并提示先去
  「Agent 配置」页创建，不再静默回落内置 `act` 创建会话。
- `@` 菜单候选集按模式取值：Agent 模式与下拉同源（仅注册卡）；Operator
  模式保持内置在前 + 注册卡（act/plan 切换与自定义 Agent 提及不变）。
- 顺带修复 busy 会话切换注册卡的隐藏 bug：`switchAgent` 原先对自定义名拼
  `/<name>` 控制头（runner 只认 `/act`、`/plan`、`/agent <name>`，会被当普通
  prompt 文本）；现仅 act/plan 用独立头，其余目标（含 command）统一走
  `/agent <name>`（`BUILTIN_AGENT_HEADS`，`spa/src/agents/builtins.js`）。

## 涉及文件

- `crates/web/spa/src/chat.jsx` — 下拉候选集拆分、模式感知的默认值/兜底、
  发送门禁、busy 切换控制头。
- `crates/web/spa/src/agents/builtins.js` — 新增 `BUILTIN_AGENT_HEADS`；
  `mergeBuiltinPrimaryAgentCards` 语义收敛为 Operator 模式切换面专用。
- `crates/web/src/api_agents.rs` — `GET /api/agents` doc 注释对齐（端点本就
  只返回注册卡，无行为变化）。
- `agents/web/index.md` — chat 页段落与 run_mode 徽标描述同步。

## 验证

- SPA Vitest `src/chat/chatMode.dom.test.jsx`：14 passed（含新增用例「下拉
  只列 primary 注册卡、内置永不出现」「配置为空时发送被拦截并提示」；改写
  默认创建/切换/徽标三用例为注册卡语义）。
- SPA 全量 Vitest：114 files / 856 tests passed。
- `cargo check -p opencoder-web` 通过（仅注释改动）。
- `npm run build` 重建 `crates/web/spa/dist`（编译期嵌入，gitignore）。
