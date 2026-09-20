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

## 误导源更正（同日第二轮）

首轮把 Agent 模式「执行 Agent」下拉收敛为仅注册卡后，代码里仍残留把内置
runtime 角色与注册 Agent 卡混为一谈的注释/文案（排查确认
`web_agent_resources.rs:621` 的 `"workflow"` 只是 skill 资源包名，不触及本
议题）。逐一掐灭，除一处行为一致性修正外均为注释级：

- SPA 4 处：
  - `spa/src/agents/builtins.js` 头注释重写——声明两类分界（builtin 是
    agent loop 的 runtime 执行角色；注册卡经 opencoder-agent 注册、NFS
    分发、`GET /api/agents` 下发，是完全不同的两类物），并明确本层 merge
    的合法用途仅限调度执行器目标（todoEditor allowed、brain
    targetOptions）与 Operator 会话切换面；Agent 模式下拉禁用合并。
  - `spa/src/commandMenu.js` `agentsToCommands` doc——删除过时的
    "(+ builtin primaries merged in by chat.jsx)" 一刀切表述，改为入参按
    调用方模式区分（Operator 并入内置、Agent 仅注册卡）。
  - `spa/src/todoEditor.jsx`、`spa/src/brain/targetOptions.js`——各补半句：
    此处选的是调度执行器目标，不是 Agent 对话能力选择面。
- Rust 3 处：
  - `web/src/api.rs` `post_agent`——注释补 `command`（primary 切换面实为
    act/plan/command）并注明 workflow 排除口径；错误文案改为 "expected a
    builtin primary agent (act/plan/command) or a registered file agent
    name"。
  - `core/src/agent/mod.rs` 模块头——加两类条目分界一句（builtin=loop 的
    runtime 角色，含 subagent/command/workflow 调度器；file=注册能力卡；
    解析层统一仅为实现细节）。
  - `tui/src/agent_menu.rs`——头部与 `available_primary_agents` doc 说明
    内置行=runtime 会话切换目标（operator 语义）、file 行=注册能力卡，与
    SPA Agent 模式下拉的「仅注册卡」分属两个面。
  - （`web/src/api_agents.rs` `list()` doc 已在首轮对齐两类消费方表述，
    本轮无需再动。）
- 行为一致性修正（唯一的行为改动）：`post_agent` 校验从 `is_primary()`
  收紧为 `is_primary() && name != "workflow"`，对齐 `GET /api/agents` 的
  primary 计算与 TUI picker——`workflow`（TODO 内部调度器）不再被接受为
  会话切换目标，返回 400 unknown agent（与 sandbox/explore/build 同路）。

### 验证（第二轮）

- `running_mode_gate.rs` 的
  `agent_switch_accepts_plan_and_rejects_legacy_sandbox` 内新增
  `{"value":"workflow"}` → 400 断言（锁定排除行为与零足迹）；`cargo test -p opencoder-web --test running_mode_gate --test
  web_contract --test switch_broadcast --test agent_model_toctou` 全绿
  （7+15+2+2 = 26 passed）。
- SPA 全量 Vitest：114 files / 856 tests passed（注释级改动，基线不变）。
- `cargo check -p opencoder-core -p opencoder-tui` 通过；四个改动的 Rust
  文件 rustfmt（edition 2021）clean。
- 记忆同步：`agents/web/index.md`（commandMenu 行补 `@` 候选集模式分界
  与切换端点 workflow 排除）、`agents/core/index.md`（registry 两类条目
  分界）。
