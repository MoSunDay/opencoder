Commit: c2bd85c234ea2394536308dd63c1122aa670ebc2

# 会话交互切换为 Operator 执行 + Operator 页签只读化 + Harness 新建档案弹窗

## 背景

- 会话交互页(`chat.jsx`)此前以 `ExecutionKind::Agent`(runc 容器内 agent loop)创建会话;Operator 是 worker 节点宿主机进程直跑 agent loop 的通道,且平台角色门控本就允许 user 角色提交 operator 执行。
- 「Agent 配置 → Operator」页签的「操作」列与 `launchModal.jsx` 启动链路与会话页能力重复;Harness 管理页的「Wrap 参数统一管理」Alert 与内联 Input 新建档案交互待收敛。

## 变更

### 1. 会话语义切换:Agent → Operator(control)
- `crates/control/src/api/session.rs::create`:会话执行 `kind` 改为 `ExecutionKind::Operator`,默认 id 前缀 `agent-{ulid}` → `operator-{ulid}`(与 `CreateExecution::validate` 的 kind 前缀约束一致)。
- `session.rs::summaries`(即 `/api/nodes/:id/dialogs` 数据源)与 `api/compat/sessions.rs::task`(旧任务 API)同步 Operator;compat `metadata` 的节点选择口径同步 Operator(models/skills 服务于会话页)。
- 存量 Agent 会话不再出现在会话交互侧栏,仍可在「执行记录」查看/继续。

### 2. 前端会话页与节点口径
- `spa/src/chat.jsx`:`newId('agent')` → `newId('operator')`;`canUseNode(nodes, nodeSel, 'operator')`。
- `spa/src/chatSidebar.jsx`:节点选择器 `explicitNodeOptions(nodes, 'operator')`。
- 相关 DOM 测试的节点 fixture 补报 `operator` kind。

### 3. Operator 页签只读化
- `spa/src/operators/nodeTable.jsx`:删除「操作」列与 `operable()`;`panel.jsx` 移除 `LaunchModal`/`ExecutionDetail` 状态,仅保留说明 + 节点表。
- 删除死代码:`operators/launchModal.jsx`、`operators/launchModal.dom.test.jsx`。

### 4. Harness 管理页
- 删除「Wrap 参数统一管理」Alert(`fields.jsx` 启动时的提示文案保留)。
- 「新建配置档案」由内联 Input+按钮改为 antd `Modal + Form`(okText=创建、confirmLoading、destroyOnHidden):名称(required + `^[A-Za-z0-9][A-Za-z0-9._-]{0,47}$` + 重名校验)、模型(--model)、环境变量(--envs)可选(复用 `wrapSettings` 校验);提交 `PUT /api/harnesses/codex/profiles/:name`(后端 upsert),成功后刷新列表并选中新档案,失败保留表单可重试。选中档案的就地编辑表单不变。

### 文档
- `agents/web/index.md`:会话页与 Operator 页签描述同步。

## Validation

- `crates/control`:265 项测试通过(更新 `sessions_relay.rs`、`sessions_compat_extra.rs`、`compat_nodes.rs` 的 id/kind 断言与 `put_index` fixture:agent → operator)。
- SPA 全量:`npx vitest run` 103 个测试文件、737 项通过(更新 `operators/panel.dom.test.jsx`——删除启动按钮/launch flow 用例,新增只读断言;`harness/management.dom.test.jsx` 新增弹窗创建与重名/非法名拦截用例;`chat.*`/`sidebar` 节点 fixture 补 operator kind)。
- `npm run build` + `scripts/check-spa-drift.sh`:dist 内嵌产物同步重建、无漂移。
- 受影响 e2e 同步:`tests/running_mode_switch_e2e.rs` 的显式会话 id `agent-plan-clear-handoff-e2e` → `operator-` 前缀,`cargo test -p opencoder --test running_mode_switch_e2e` 2 项通过。
- `cargo test -p opencoder-control`:265 项通过;`cargo test -p opencoder-server`:3 项通过。
- `cargo test -p opencoder-worker -p opencoder-cli` 本轮多次尝试均因共享 target 目录构建锁被其他并行会话长时间占用而未能完成;这两个 crate 本次未改动(上次基线 12 个用例全部通过),并发会话的 workspace 级回归已覆盖它们。
- workspace 全量:共享构建目录被并发验证持续占锁,本轮以同工作树 `cargo test --workspace` 门禁日志为旁证(5263 passed,唯一失败为上述 `running_mode_switch_e2e` 的 `agent-` 前缀断言,已修复);锁空闲时段建议补跑一次 `cargo test --workspace` 收口。
