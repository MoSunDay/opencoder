Commit: 7177f7987d3c3cdc7ed17864a4ffcf347bf4d182

# 大脑分层能力画布（v4）：领域 → 持久化 → 节点 → 控制面 → CLI → Web 的迭代收口

本轮迭代新增 `schema_version: 4` 的「分层能力画布」运行，把能力编排从 v3 的「逐节点决策」升级为「先按依赖切层、按层决策、整层终结后放行下一层」：计划在准入时按依赖拓扑切层（kahn，不依赖 JSON 顺序）；节点只在所属层被决策时创建一次尝试；层屏障要求整层终结（done/failed/cancelled）后才放行下一层；`retry.max_attempts`（1..=5，缺省 2）只重试失败节点而不重复层决策；最后一层终结后用空 `nodes` 的收口上下文完成根运行并冻结 `summary`。上限：节点 ≤ 256、单层宽度 ≤ 32、嵌套深度 ≤ 3、`max_rounds` 1..=32（缺省 32）。

三件不可协商的事在本轮落定为可回归的断言：`schema_version: 3` 的线协议与行为逐字节不变（v4 只走新增的 `==4` 分支，v3 路由、回执、输出适配与 projection 语义原样保留）；Store `SCHEMA_VERSION` 未被 v4 推动（合并线上为 28，来自同批合并的发布提交自身；v4 三张表由 `brain_layered` 加性迁移创建）；层划分从不落库，一律由 `opencoder_brain::layered::layers` 重算，投影与计划不会漂移。未知 `schema_version`（2、5、缺失、字符串）在准入即显式报错，绝不回落到 v3/v2 写路径。

本轮交付横跨六层。逐层细节见同目录三份切片回执：`brain-layered-v4-worker.md`（领域 + 持久化 + 节点）、`brain-layered-v4-control.md`（控制面准入/读取/投递）、`brain-layered-v4-e2e.md`（CLI 读取面 + 进程级 e2e）。本文是全轮收口：合并全部测试清单，并附通过全部 gate 的实跑回执。

## 变更摘要

- **领域与线协议**：`crates/brain/src/layered/`（`levels`/`context`/`decide`/`activation`/`terminal`/`validate`/`prompt`，纯函数无 I/O）承载层划分、层上下文与祖先绑定、层决策、层屏障与准入；`crates/core/src/brain/layered/`（`plan`/`run`/`decision`）承载 LOCKED DTO（`LayeredRequest`/`LayeredPlan`/`LayeredRun`/`LayeredOperation`/`LayeredContext`/`LayeredDispatchIntent`/`LayeredTerminalEvent` 等）。`LayeredRun.layer` 是已完成层数，正在决策的层恒为 `run.layer + 1`。
- **持久化**：新增 `brain_layered_runs` / `brain_layered_operations` / `brain_layered_events`（`crates/store/src/libsql_store/brain_layered/schema.rs`）与 `brain_layered` / `commit_brain_layered` / `brain_layered_events` 三个 trait 方法；投影写入按 generation 栅栏且原子，重试落成独立 operation，终态不可变。
- **节点侧**：`crates/worker/src/brain/v4/`（`state`/`parent`/`output`/`outbox`/`api`/`run`，与 v3 同构）实现 `layered_wake`/`layered_dispatch`/`layered_cancel`/`layered_terminal` 动作面；每 generation 只发一次 wake 且 ack 后关闭；层上下文安装后节点自行重入队做一次层决策；层屏障与栅栏只留在节点。
- **控制面**：`crates/control/src/api/brain_runs/v4/`（`api`/`catalog`/`delivery`/`gateway`/`read`/`request`/`runtime`/`view`）补齐准入分派、幂等（同一 intent 重放回执、异 intent 409）、`layered_` 前缀动作投递与回执转发、`GET /api/brain/runs/:id/layered` 与 `/layered/rounds/:round`；能力冻结为 `BRAIN_V4`，v4 只落在广告 `brain_scheduler_v4` 的节点；控制面不持层划分与 generation。
- **CLI**：`brain runs create --json` 只放行显式 3|4（2/5/缺失/字符串在计划期报错）；新增 `brain runs layered <id>` 与 `brain runs layered-round <id> <round>`；v3 `runs round`/`runs get` 路径不变。
- **Web 画布**：`crates/web/spa/src/brain/workbench/layered/`（`model.js`/`canvas.jsx`/`rounds.jsx`/`events.jsx`/`run.jsx`/`style.css`）每层一列渲染、层屏障进度、重试徽标、分层事件与层决策明细；只认显式 `schema_version 4`，v3 运行仍走 v3 快照与 `/view`，绝不请求 `/layered`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 层拓扑（kahn 分层、非 JSON 顺序）、派发覆盖整层并固定上游绑定 | `kahn_levels_follow_edges_not_json_order`、`dispatch_covers_the_layer_and_fixes_bindings` | `crates/brain/tests/layered/validate.rs` |
| 计划/请求准入：未知字段、外来 todo、能力恰好一个、宽度与深度上限、未知 schema、失联父运行 | `plan_rejects_unknown_fields_and_foreign_todo`、`node_requires_exactly_one_capability_and_unknown_edges_fail`、`request_rejects_unknown_schema_and_lost_parent` | `crates/brain/tests/layered/validate.rs` |
| 层内并发与层屏障、失败重试与终态失败、非祖先绑定拒绝、上一尝试迟到终态被忽略 | `parallel_nodes_in_layer_then_next_layer`、`retry_schedules_next_attempt_then_fails_run`、`binding_to_non_ancestor_rejected`、`late_terminal_from_previous_attempt_ignored` | `crates/brain/tests/layered/barriers.rs` |
| 完成要求整层收口并冻结 summary；pause/cancel 停止派发 | `completion_requires_every_layer_and_freezes_the_summary`、`cancel_and_pause_commands_stop_dispatch` | `crates/brain/tests/layered/terminal.rs` |
| 投影按 generation 栅栏且原子、重试落成独立 operation、终态不可变且重开仍在、未知/伪造运行拒绝、Store `SCHEMA_VERSION` 未被 v4 推动 | `layered_projection_is_atomic_and_generation_fenced`、`retry_attempts_are_stored_as_separate_operations`、`terminal_attempt_is_immutable_and_survives_reopen`、`unknown_run_reads_as_none_and_forged_operations_rejected`、`store_schema_version_is_untouched` | `crates/store/tests/brain_layered_v4.rs` |
| 每 generation 只发一次 wake，ack 后关闭，暂停/恢复换新 generation | `root_emits_one_layered_wake_until_control_acknowledges_it` | `crates/worker/tests/brain_scheduler_v4.rs` |
| 安装层上下文后节点自行重入队做一次层决策；派发帧与 durable intent 一致、伪造操作 409、ack 后停止重放 | `layered_context_requeues_the_idle_root_for_its_layer_decision` | `crates/worker/tests/brain_scheduler_v4.rs` |
| 失败尝试按 `retry.max_attempts` 重试（不重复决策、不失败运行），层屏障放行后下一层绑定上游 execution | `failed_attempt_retries_then_the_barrier_opens_the_next_layer` | `crates/worker/tests/brain_scheduler_v4.rs` |
| 空节点收口上下文完成根运行、summary 绑定进结果、终态不再派发 | `closing_context_completes_the_root_and_binds_the_summary` | `crates/worker/tests/brain_scheduler_v4.rs` |
| 视图/层明细读节点投影：键、层划分重算、越界层 404 | `layered_view_and_rounds_read_the_node_projection` | `crates/control/tests/e2e/layered_api/surface.rs` |
| 不跨版本服务：v3/未知运行读 layered 404，v4 运行读 v3 呈现 409 | `layered_routes_never_cross_serve_another_schema` | `crates/control/tests/e2e/layered_api/surface.rs` |
| 事件流与 `?offset=` 快照都委托节点投影 | `layered_events_and_snapshot_routes_delegate_for_v4_runs` | `crates/control/tests/e2e/layered_api/surface.rs` |
| 准入要求 v4 广告（v3-only 节点 503 并点名能力）、冻结能力 scope、同 intent 重放回执 | `layered_admission_requires_the_v4_advertisement_and_freezes_the_scope` | `crates/control/tests/e2e/layered_api/mod.rs` |
| 未知/历史 `schema_version` 显式报错，不建索引、不触发模型 | `unknown_or_legacy_schema_versions_are_explicit_errors` | `crates/control/tests/e2e/layered_api/mod.rs` |
| 请求成形：环与未知能力在准入即 400，不进入选点 | `layered_create_requires_a_well_formed_request` | `crates/control/tests/e2e/layered_api/mod.rs` |
| 嵌套准入：深度 ≤ 3 且必有 parent，合法嵌套冻结父绑定 | `layered_nesting_is_bounded_and_requires_its_parent` | `crates/control/tests/e2e/layered_api/mod.rs` |
| 生命周期命令只放行 pause/resume/cancel，其他 400；未知/历史运行 409 | `layered_commands_forward_only_the_three_lifecycle_actions` | `crates/control/tests/e2e/layered_api/commands.rs` |
| wake 只确认自己激活的 generation；迟到 wake 不吃掉新一轮 | `layered_wake_acknowledges_the_generation_its_activation_admitted`、`stale_layered_wake_does_not_acknowledge_a_new_ready_generation` | `crates/control/src/transport/layered_tests.rs` |
| 能力探针：v4 运行与 `brain_layered` 子执行都要 `brain_scheduler_v4`，旧探针不冒充 | `layered_runs_require_the_v4_advertisement`、`old_positive_probes_do_not_advertise_new_protocol_operations` | `crates/control/src/api/executions/capabilities.rs` |
| CLI 内层激活按 v4 上下文发层契约并写回决策；空 `nodes` 走收口指令 | `layered_context_sends_the_layer_contract_and_writes_the_decision`、`closing_layered_context_uses_the_closing_instruction` | `crates/ctl/src/cmd/brain/ontology/tests.rs` |
| CLI 计划映射：create 版本门禁、v4 读取路径、v3 round 路径不变 | `run_create_accepts_schema_versions_three_and_four`、`run_reads_map_to_the_locked_paths` | `crates/ctl/src/cmd/brain/ontology/tests.rs` |
| CLI 解析端到端：`brain runs create --json` 的 3\|4、`brain runs layered` / `layered-round` 路径与退出码 | `brain_run_create_accepts_v3_and_v4_and_layered_reads_match_the_contract` | `crates/ctl/tests/parse_project_brain_agents.rs` |
| 裸 `/api/executions` 提 Brain 仍 409 且零模型调用；缺失/2/5 版本 409 且 `/layered` 404；非法画布（坏 id、未知能力、环、深度 4、深度 1 无父）400 且不建运行 | `raw_brain_submissions_stay_rejected_for_the_layered_canvas`、`unknown_schema_versions_never_fall_back_to_a_writer`、`invalid_canvases_are_rejected_before_any_dispatch` | `tests/brain_layered_e2e/admission.rs` |
| v4 视图键与层明细、越界层 404、跨版本 404/409、命令白名单 400、CLI 与 HTTP 一致 | `layered_view_and_rounds_read_a_real_projection` | `tests/brain_layered_e2e/surface.rs` |
| 真实 runc 画布跑到 `completed`：层屏障先等待后放行、逐层事件序、子执行 `brain_layered` 绑定与 `scheduler_output`、收口 summary、CLI 与模型调用计数 | `layered_canvas_holds_the_barrier_then_completes_through_the_closing_activation` | `tests/brain_layered_e2e/canvas.rs`（无 runc 时 SKIP 运行段；本机 `/usr/bin/runc` 可用，运行段真实执行） |
| 画布模型：只认显式 `schema_version 4`、八种运行阶段、层分组与节点汇总、重试链滚动、层屏障进度、每层一列与绑定标签、事件与层决策明细归一化 | `只认显式 schema_version 4…`、`phase / terminalPhase 覆盖 LOCKED 的八种运行阶段`、`layers[i] 是第 i+1 层…`、`层汇总按最新尝试滚动…`、`重试链保留全部尝试…`、`层屏障进度 = 已完成层 / 总层数…`、`画布每层一列…`、`事件按 seq 升序…`、`roundDetail 归一化层决策明细…` | `crates/web/spa/src/brain/workbench/tests/layered/model.test.js` |
| 画布渲染与演进：分层画布/屏障/重试徽标渲染并按需拉层明细、事件帧触发重取、无事件流每 3 秒轮询 `/layered`、v3 走 v3 快照与 `/view` 绝不请求 `/layered`、未知 schema 显式报错且缺 `run` 失败关闭、基础快照 404 回落 `/layered` | `渲染分层画布、层屏障、重试徽标与分层事件…`、`v4 事件帧触发分层视图重取…`、`没有事件流也不会冻结…`、`v3 运行仍走 v3 快照 + /view…`、`未知 schema_version 显式报错…`、`基础运行快照 404 时回落到 /layered…` | `crates/web/spa/src/brain/workbench/tests/layered/layered.dom.test.jsx` |
| v3/v2 兼容回归（v3 唤醒、上下文重入队、失败折叠、端到端往返；v2 计划解析与元数据保留） | `root_emits_one_scheduler_wake_until_control_acknowledges_it`、`scheduler_context_requeues_idle_root_for_node_decision`、`failure::child_failure_cancels_inflight_sibling_without_another_model_decision`、`control_round_trip_dispatches_child_and_waits_for_terminal_barrier`；`saved_plan_runs_resolve_scope_inputs_and_persist_round_evidence` 等 | `crates/worker/tests/brain_scheduler_v3.rs`、`crates/worker/tests/brain_scheduler_plans.rs` |
| v3 读路径未被 CLI 与准入改动影响（`/api/executions` 提 Brain 仍拒、v2 写只读且要求显式 v3 schema） | `lifecycle::raw_brain_submissions_are_rejected_in_favor_of_v3_runs`、`lifecycle::v2_run_writes_are_read_only_and_require_explicit_v3_schema` | `tests/brain_e2e/main.rs` |

- 全量回归：`cargo test --workspace` → **5570 passed / 0 failed / 8 ignored**（439 个测试目标，脚本判定 `EXIT=0`）。收口末次复跑（含 `crates/brain/tests/layered/{barriers,terminal}.rs` 拆分与文档定稿后）与拆分前一次逐项一致：同 439 个测试目标、5570 passed、0 failed。8 个 ignored 全部是既有的手工门（3 × runc 沙箱、mounted CLI 容器、Chromium 浏览器验收、2 × 挂载/NFS 特权、NFS 只读节点快照），非本轮新增跳过。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（exit 0）；新增测试文件落地后复跑仍为 exit 0、日志零 `warning`。
- 构建：`cargo build --workspace` → 零错误（exit 0）。
- 格式：`rustfmt --edition 2021 --check` 对本轮全部 79 个新增/改动 Rust 文件无差异。（已知遗留：`crates/web/tests/dag_api.rs` 在本次迭代前就已未格式化，本轮未触及该文件。）
- 行数 gate：本轮新增文件最大 365 行（`crates/control/src/api/brain_runs/v4/delivery.rs`），全部 ≤ 400 行；新增目录最多 9 个文件（`crates/control/src/api/brain_runs/v4/`），全部 ≤ 10 个。
- 安全 gate：diff 与未跟踪文件无硬编码密钥/凭证/连接串（仅测试夹具 `"api_key":"fixture"` 与 `token_hash` 测试符号）；`crates/store/src/libsql_store/schema.rs` 的 `SCHEMA_VERSION` 为 28（发布提交自身携带，v4 未推动）；`crates/store/src/bundle.rs` 未被改动。
- v4 定向实跑：`opencoder-brain --test layered` 11 passed；`opencoder-store --test brain_layered_v4` 5 passed；`opencoder-worker --test brain_scheduler_v4` 4 passed；`opencoder-control --test e2e -- layered` 8 passed（e2e 全量 204 passed）；`opencoder-control --lib layered` 3 passed；`--test brain_layered_e2e` 5 passed；`-p opencoder-cli` 100 passed（13 个测试目标）。
- v3/v2 回归定向实跑：`--test brain_e2e` 2 passed；`--test dag_e2e` 17 passed；`-p opencoder-worker --test brain_scheduler_v3` 4 passed。
- SPA：`bash scripts/build-spa.sh` 成功；`bash scripts/check-spa-drift.sh` → `spa dist: no drift`（提交的 `dist/` 与源码重建逐字节一致）；`npm test`（`crates/web/spa`）→ 122 个测试文件 / 907 passed。
- 契约交叉核对：`schema_version: 3` 相关路径的改动全部是 `== 4` / `Some(4)` 加性分支或新模块/新路由（`crates/worker/src/brain/{api,outbox,output,wake}.rs`、`crates/worker/src/workloads/mod.rs`、`crates/control/src/api/brain_runs/*`、`crates/control/src/api/executions/capabilities.rs`、`crates/control/src/transport/mod.rs`）；v3 分支逐字节保留。
- 测试基线说明：本轮未单独跑改动前的 workspace 全量基线（隔离 worktree 仅一份），以 v3/v2 定向套件（`brain_e2e`、`dag_e2e`、`brain_scheduler_v3`、`opencoder-cli`）作为兼容回归基线，全部通过且计数与切片回执一致。

## 兼容与边界

- v3 未改语义：外层接缝按 `schema_version` 分支（`==3` 走 v3、`==4` 走 v4），v3 的路由、回执、输出适配与 projection 语义逐字节保留。
- Store `SCHEMA_VERSION` 为 28（发布提交自身携带，v4 未推动），v4 表由加性迁移创建；控制面不写层划分、不持 generation；层一律重算。
- 迭代中文件 > 800 行的既有偏差（本轮只有加性增长，未新增超限文件）：`crates/store/src/store.rs` 923→944（+21）、`crates/store/src/libsql_store/impl_store.rs` 868→891（+23）、`crates/store/src/libsql_store/schema.rs` 822→823（+1）；三者在本轮开始前已超 800 行，拆分属独立重构，不在本轮范围。

## 相关文档

- [features/brain/index.md](../../brain/index.md) — 分层能力画布的用户可见行为
- [agents/brain/index.md](../../../agents/brain/index.md) — 领域模块索引（`src/layered/` 与 `tests/layered/{validate,barriers,terminal}.rs`）
- [agents/worker/index.md](../../../agents/worker/index.md)、[agents/control/index.md](../../../agents/control/index.md) — 节点与控制面 v4 接缝
- [agents/ctl/index.md](../../../agents/ctl/index.md)、[agents/web/index.md](../../../agents/web/index.md) — CLI 读取面与画布前端
- [agents/store/index.md](../../../agents/store/index.md)、[agents/core/index.md](../../../agents/core/index.md) — v4 持久化与线协议 DTO
- [docs/brain-orchestration.md](../../../docs/brain-orchestration.md) — v4 层屏障契约
- 切片回执：[brain-layered-v4-worker.md](brain-layered-v4-worker.md)、[brain-layered-v4-control.md](brain-layered-v4-control.md)、[brain-layered-v4-e2e.md](brain-layered-v4-e2e.md)
