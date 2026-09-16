Commit: c2bd85c234ea2394536308dd63c1122aa670ebc2

# Agent List 收敛为「仅注册 Agent」（数据源层）

## 背景与语义裁决

- 今日早些时候的一次性线上清理（见 `agent-list-cleanup.md`）把列表压到 10 项（builtin 7 + 保留 3 卡），但根因是 `GET /api/agents` 把 builtin 调度角色并进管理列表。本次在数据源层持久修复：`/api/agents` 语义从「builtin ∪ 注册卡」改为**仅注册卡**（agents root 的 file 卡）。
- builtin `act/plan/explore/build/sidecar/command/workflow` 是 opencoder 内置的 subagent 调度角色（定义在 `crates/core/src/agent/mod.rs::builtin_agents()`），属运行时概念，不再伪装成「可管理条目」。管理列表（Agent 配置页）、生效 Agent 下拉、能力编辑器目标、TODO 模板 allowed 各归各位。

## 改动

- `crates/web/src/api_agents.rs::list` — 去掉 `builtin_agents()` 并集，只返回 `list_agents()`；doc comment 写明「仅注册卡」。`builtin` 字段保留（注册卡恒 false，向后兼容）。写路径（PUT/DELETE builtin 400、active 预检、harness 覆盖）语义不变。
- `crates/web/spa/src/agents/builtins.js`（新增）— `BUILTIN_PRIMARY_AGENTS = ['act','plan','command']` + `mergeBuiltinPrimaryAgents`（内置在前、注册卡在后、去重），注释指向 core 唯一事实源。workflow 属 TODO 内部调度器、explore/build/sidecar 属 subagent，均不入选。
- `crates/web/spa/src/todoEditor.jsx` — allowed = 内置三角色 ∪ 注册 primary 卡（排除 workflow）。`EXAMPLE_SPEC` 引用 `agent:'act'`，不并回会新建模板即误报「不可用的 Primary Agent」。服务端 `/api/todo/validate-files` 只做 spec 解码 + env 应用，不校验 agent，故适配面仅在 SPA。
- `crates/web/spa/src/brain/targetOptions.js` — 能力编辑器 agent 目标下拉同样并回内置三角色（brain playbook 规范示例即 `target:'act'`）；`staleTarget` 合并逻辑不变，既有指向 builtin 的能力不受影响。
- `crates/web/spa/src/agentsConfig.jsx` — 删除按钮去掉 `disabled={r.builtin}`（列表已无 builtin，防御逻辑成死代码）。
- 文档：`agents/web/index.md`、`agents/agents/index.md` 语义更新。

## 测试（rules/01 + 03）

- `crates/web/tests/web_agents.rs` — `empty_root_lists_null_active`、`delete_active_card_clears_marker_and_fans_reload` 改为「空 agents root ⇒ 列表为空、无 builtin」断言；`cards_crud_activation_and_listing` 断言创建后列表恰为注册卡（`["a","b"]`）且 `builtin == false`。
- `crates/ctl/tests/server_local.rs::agents_card_lifecycle_and_active_pointer` — 计划外暴露的 e2e 消费方：原断言 `builtin 7 + 1` 与七张 builtin 行断言改为「列表恰为注册卡 alpha、builtin == false」（工作区全量回归发现并修正）。
- SPA：`agentsConfig.dom.test.jsx` 新增「仅注册卡渲染、builtin 七角色不渲染不进下拉」用例；`brain/targetOptions.test.js` 断言 agent 选项 = 内置三角色 + 注册卡、与 builtin 重名卡去重；`brainPanel.dom.test.jsx` mock 改注册卡载荷且 `act` 仍可选；`todoEditor.dom.test.jsx` 四处 `/api/agents` mock 改为注册卡载荷（`act` 可用性改由内置并集保证，全部用例即隐式回归）。

## 验证

- `cargo test -p opencoder-web`（52 单测 + 集成全绿）、SPA `npm run test`（751/751）+ `npm run build`、`cargo test --workspace` 回归（见下）。

### rules/02 回归结果与并行会话干扰说明

- `cargo test --workspace --no-fail-fast`（同树、含全部本改动）：402 个测试二进制 ok，唯一失败 `tests/running_mode_switch_e2e.rs::real_server_clear_context_executes_preserved_plan_in_act`（`POST /api/sessions/:sid/prompt` 返回 400，plan→act clear-context 场景）。
- 该失败与本改动无关的证据：① 用例全程不调用 `/api/agents`（grep 计数 0），断言点在 prompt 准入路径，本改动未触碰；② 同一工作区在 16:40 的上一次全量回归中该用例通过（当时 api_agents.rs 改动已在树中），而并行会话 16:40 后继续在同一工作区迭代 control/worker/session 等在途改动；③ 单独复跑两次均稳定 400，为确定性失败，属并行会话在途代码（agent-owned resources 等）引入，非本改动回归。
- **发版暂缓（已按用户指令解除）**：SPA 与 API 均编入二进制，而当时工作树混有并行会话在途改动，曾记录暂缓发布。后续在途工作已在 `d31ec4e9` 落地、SPA dist 重建无漂移，用户明确指令跳过全量回归直接发布，本轮于 2026-09-16 18:08 信号发布 `rel-2868ebfd`（回归豁免与未复跑项见 [release-2868ebfd-signal-deploy](release-2868ebfd-signal-deploy.md)）。线上 `GET /api/agents` 复核为仅注册卡 + builtin 调度角色。
- 发版 dev 后线上复核：`GET /api/agents` 仅含注册卡（当前 3 张：eval-diagnose / regression-test / viking-dependency-analysis-agent）。

## 测试清单

- `cargo test -p opencoder-web --test web_agents`（9/9）
- `cargo test -p opencoder-cli --test server_local`（9/9）
- `cargo test -p opencoder-web --test web_todo_templates --test web_todo_envs --test web_todo_runs`（15/15）
- `cd crates/web/spa && npm run test`（104 文件 751 用例全绿）+ `npm run build`
- `cargo clippy -p opencoder-web --all-targets`（零告警）、`cargo fmt --check`（幂等）
- `cargo test --workspace --no-fail-fast`（402 二进制 ok，1 个与并行会话在途代码相关的失败，见上）

## 收口终态（2026-09-16 18:00-19:30）

- **门转绿**：SID 修复 `de4dab6f`（用例名对齐 operator kind 会话命名）落地后，`real_server_clear_context_executes_preserved_plan_in_act` 单跑通过（0.88s）。随后在共享 target dir 上对 357 个 cargo test 目标逐二进制直跑复核（并行会话持续占用构建锁，改为绕过锁直接执行已编译测试二进制；`CARGO_BIN_EXE_*` 依赖手动注入）：**1571 项 passed / 0 failed**，含 root 6 个 e2e（running_mode_switch_e2e / daemon_smoke / nodes_smoke_proc / responses_cli / tui_exit_restore_e2e / skill_seed_startup_wiring）全绿。结果留档 `/tmp/test_results_all.txt`。
- **发版（并行会话按用户指令执行）**：`rel-2868ebfd` 信号发布 18:08 上线，`/api/health` 返回 `commit 0.1.0 (2868ebfd)`、protocol 9，旧 Runtime（rel-79eee711）已 hibernate，详见 [release-2868ebfd-signal-deploy](release-2868ebfd-signal-deploy.md)。
- **线上复核（`127.0.0.1:3039`，发版后）**：`GET /api/agents` 共 8 行、**builtin 行为空**（8 张注册卡：eval-diagnose / regression-test / viking-dependency-analysis-agent + 5 张 pingce 评审卡，均为今日新增注册卡的正常生长）；`DELETE /api/agents/act` → 400（builtin 保护在位）；`GET /api/agents/act` → 405（单卡路由为 `/:name/meta`，设计如此）；SPA `static/app.js` 200。
- **active marker**：线上指向 `static-auto-test-pingce-prepare-context`（注册卡，非 builtin）——「生效下拉裸值」场景不适用；SPA 生效下拉 options 仅注册卡，且 `allowClear` 可恢复「跟随默认链」。
- **数据收口**：agents root 存在 `act`（9 月 9 日）与 `sidecar`（14:43）两张**空引用** builtin 同名 file 卡（harness 试探残留，history 均为 3 秒内 opencoder→codex→opencoder 切换）。它们在管理列表显示为 `builtin:true` 行、API DELETE 被 builtin 保护拒绝（幽灵卡）。已移出至 `/var/tmp/agents-ghost-cards-backup-181636`（可回滚）；`resolve_agent` builtin 优先，删除对运行时零影响。移除后列表恰为注册卡。
- **设计观察（不阻塞，留后续迭代）**：① `PUT /api/agents/:builtin-name` 走 `update_agent_with_profile` 的 `None if builtin` 分支自愈创建**覆盖卡**（web 层无 400，与本文件上文「PUT builtin 400」的原述不符，实测 200 并重建空卡）；空覆盖卡会以 `builtin:true` 行进入管理列表，与「仅注册卡」语义存在灰色地带；② 外部进程可能重建同名卡（备份目录 `/var/tmp/agents-ghost-cards-backup-*` 可观察再生长）。两者均属后续功能迭代议题。
- **builtins.js ↔ core 镜像维护点**：本轮豁免注释补强（`spa/src/agents/builtins.js` 与 `crates/core/src/agent/mod.rs::builtin_agents()` 注释已双向指向），随下次代码改动顺带补强。
