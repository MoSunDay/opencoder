Commit: 2868ebfd3fbd5f492d48cdc7c3282ff0df791263

# 信号发布 rel-2868ebfd（agent 自有资源、workdir 调度、TUI Say 无损等一批）

2026-09-16 18:08 信号发布上线 `rel-2868ebfd3fbd5f492d48cdc7c3282ff0df791263`（前序 `rel-79eee711`），`scripts/platform/deploy.sh --signal --wait-seconds 300` 由当前 Server 触发独立发布作业，`phase=complete`、无失败记录。发布携带 `d31ec4e9` 全量特性：节点调度 workdir（control/worker/web + `scheduling_workdir.rs`）、Agent 注册卡收敛与自有资源编辑器（`crates/agents/src/resources/`、`GET/PUT /api/agents/:name/resources/:cat`）、DAG 单步执行记录（SSE steps events + SPA `dag/step` 抽屉）、team 话题推进合约 e2e、会话 operator kind 与 Operator 页签只读、TUI Say 无损传递（worker/delivery.rs）、Web 会话工作台（act/plan 节点暂存、会话悬停删除、TODO review conversation 拆分）。

发布前置处理（同日）：`de4dab6f` 将 `real_server_clear_context_executes_preserved_plan_in_act` 用例 SID 对齐 operator kind 命名（旧命名下该用例稳定 400，属用例未跟上新会话语义）；`2868ebfd` 在 HEAD 重建 `crates/web/spa/dist/static/app.js` 并通过 `scripts/check-spa-drift.sh` 无漂移。

## 验证与上线复核

- 构建门：`scripts/platform/release/build.sh` 全绿（四二进制 + spa_sha256 + 构建元数据一致性），bundle `/srv/releases/rel-2868ebfd3fbd5f492d48cdc7c3282ff0df791263`。
- 上线复核：`/api/health` 返回 `commit 0.1.0 (2868ebfd)`、`signal_protocol: 1`；Host 节点上报 `0.1.0 (2868ebfd)` protocol 9；全部旧 Runtime（含 rel-79eee711）`hibernated` 且 `remaining` 为空，旧 Server/Host unit 已 inactive；新 Server/Host/Runtime 三 unit active。
- `GET /api/agents` 仅注册卡 + builtin 调度角色（act/sidecar 标记 `builtin:true`），非 builtin 注册卡 8 张（eval-diagnose、regression-test、viking-dependency-analysis-agent 与 5 张 pingce 评审卡）。
- **回归门豁免说明**：本轮按用户明确指令跳过 `cargo test --workspace` / clippy 全量回归直接发布。发布前已知 `tests/running_mode_switch_e2e.rs::real_server_clear_context_executes_preserved_plan_in_act` 在旧 SID 下失败（400），修正（SID 对齐）已提交但**未复跑**；900 秒真实模型验收（`scripts/acceptance/smooth_release/live.py`）本轮未执行。上线后需人工复核 plan→act clear-context 链路与 prompt 准入路径，若出现回归按 `viking-cli ops rollback`（本仓库流程为 `deploy.sh --signal --rollback`）回退 rel-79eee711。

相关：[agent 自有资源编辑器](agent-owned-resource-editor.md)、[agent 注册卡收敛](agent-list-registry-only.md)、[DAG 单步记录](dag-step-events-query-and-console.md)、[operator kind](session-operator-kind-and-operator-tab-readonly.md)。
