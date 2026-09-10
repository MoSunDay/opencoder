Commit: (working-tree, 基于 b465f440)

# brain playbook：双轨调度第二轨——编排图域 + v25 存储 + CRUD/派发 API + 项目执行器

大脑此前只有一条调度轨：决策树把情况路由到单个能力。本迭代补齐第二轨「编排」
——playbook 用 `depends_on` 组织多步骤（每步一个执行目标 + 提示词模板），把新
情况交给一组执行器协作。固定剧本由人录入（web CRUD），动态剧本由 LLM 规划器按
situation digest 铸出并缓存；两份落地端同时接入：控制面把剧本展开成按拓扑批次
提交的 execution，本地 project 执行器把剧本跑成一组 `kind=Step` 子 run。纯域
（校验/拓扑/触发匹配）零 I/O，store 只存不透明的 `spec_json`。

## 方案

### 纯域：`crates/brain/src/playbook/`（新）

- `spec.rs`：`PlaybookSpec { schema_version, id, name, origin, trigger, steps }`，
  `origin` 与 `target`/`trigger` 均为 `kind` 标签 serde（`fixed`/`dynamic`；
  `agent|team|dag|todos|brain`；`manual|message`），LLM 原始 JSON 可直接反序列化。
  `validate` 聚合全部错误（schema/名 slug/依赖未知或自环/重复/空 target/空
  prompt/trigger 阈值 0.0..=1.0），并兜底 `MAX_STEPS=64`、`MAX_CHAIN_DEPTH=16`、
  `MAX_WIDTH=16`、`MAX_NAME_CHARS=64`、`MAX_PLAYBOOK_NAME_CHARS=120`、
  `MAX_PROMPT_CHARS=8000`；`validate_draft` 供 web 在写库前 400。
  `render_prompt` 只替换 `{situation}`（trim 后），未知占位符原样保留。
- `topology.rs`：确定性 Kahn 拓扑序（就绪集 BTreeSet 字典序）、`ready_steps`、
  `collapse_blocked`（失败步骤的传递下游一次判死）、`chain_depth`/`max_width`。
- `trigger.rs`：`manual` 永不触发（阈值 = ∞）；`message` 在相似度 ≥ threshold
  且相似度存在时触发（embed 失败 fail-closed）；`scan` 跳过 spec_json 不可解析
  的坏行；`cosine_similarity` 复用 `plan::cosine`。

### Runtime 与动态规划：`crates/brain/src/{runtime.rs,planning.rs}`

- CRUD：`create_playbook`（id `playbook-{ULID}`，origin=fixed）、
  `update_playbook`（保留 id/origin/situation_digest/created_at，替换
  name/trigger/steps，未知 id 返回 `Ok(None)`）、`get_playbook(_spec)`、
  `list_playbooks`、`delete_playbook`、`save_playbook_record`；校验错误 join 成
  一个 anyhow。
- `plan_playbook`（`PLAYBOOK_FRAMEWORK_PROMPT`）：先按 `situation_digest` 查
  最新 dynamic 缓存（命中零 LLM 调用）；空能力库回落单步 `default-act`
  （agent act，prompt 保留 `{situation}`）；否则检索候选 → 规划 → 解析（剥
  ```json 围栏）→ 纯域 validate + 候选校验（brain 步骤必须引用检索到的
  capability_id）→ 持久化。任何 LLM/契约失败是 typed `PlanGenerationFailed`。

### 存储：`crates/store`（schema v25）

- 新表 `brain_playbooks(id, name, origin, situation_digest, spec_json,
  created_at, updated_at)` + 索引 `idx_brain_playbooks_digest(digest,
  created_at)`；fresh DB 走 bootstrap CREATE，旧库走 `from < 25` 幂等迁移。
- `Store` trait 新增 `save/get/list/delete_brain_playbook` 与
  `latest_brain_playbook_for`（默认实现 bail，非 libsql 后端明确不支持）；
  libsql 实现 upsert on conflict 只替换内容、`created_at` 存活。
- `ProjectExecutorKind::Playbook`（`executor_ref` 引用剧本，内联 spec 一律
  `playbook executor takes no spec`）与 `ProjectTodoRunKind::Step`（playbook
  子尝试）；`finish_todo_run` 对 Step 不回写 todo 状态，生命周期归父 Execute 行。

### Web CRUD：`crates/web/src/api_brain.rs` + `lib.rs`

- `GET/POST /api/brain/playbooks`、`GET/PUT/DELETE /api/brain/playbooks/:id`；
  创建 201（回声 `spec` 与铸出的 id）、坏 payload 400（聚合消息原样 join）、
  未知 id 404、坏库存 spec 500；复用既有 token 门。

### Control 派发：`crates/control/src/api/brain_playbook_dispatch.rs`（新）+ `routes.rs`、`api/brain.rs`

- `GET /api/brain/playbooks`（透传列表）、`GET /api/brain/playbooks/:id`
  （404 语义与 web 一致）。
- `POST /api/brain/playbooks/:id/dispatch`：读回 spec 复核后按拓扑批次展开为
  逐步 `CreateExecution`，经 `executions::submit` 提交（202，响应含
  `batches`/`executions`）；执行 id `{kind}-pbk-{key}-{step}`（key=request_id
  截 26 字节，超长 step 名用 FNV-1a 尾哈希，保证 64 字节 id 上限内互异）；
  带 request_id 的重放按执行索引判定幂等（不重复提交）。brain 步骤解析
  `capability_target` 绑定，缺绑定回落 agent `act`，不可执行 kind 400；
  `top_k` 字段仅为二期动态重规划前向兼容接收、不消费。
- `POST /api/brain/playbooks/trigger-scan`：一期手动触发扫描——embed 入站
  文本与每个 message 触发词，输出 `matches[{playbook_id,name,similarity}]`；
  manual/坏行跳过，embed 故障 502，空 text 400；入站消息自动扫描二期落地。

### Project 执行器：`crates/project/src/executor/playbook_{drive,step}.rs`（新）

- `executor_ref` 指向剧本 id；`load_spec` 读回后再过一遍纯域 validate（纵深
  防御，引用不存在/损坏/无效都是父 run Failed 的失败输出）。
- 调度：`ready_steps` 批次并发（JoinSet），步骤启动即落一条 `kind=Step` 子
  run 行（普通插入、不 claim）；失败后 `collapse_blocked` 判死传递下游（未
  启动的步骤没有子行）；父 Execute 行独占收尾与 todo 状态；子令牌经
  `child_token` 联动取消，步骤 panic 走 `recover::converge_panicked_run`。
- `resolve_step`：`render_prompt` 渲染指令（写 `plan_md`/`draft`，清
  `executor_spec`/`active_session_id`）、`situation_for` 由目标/里程碑/标题/
  草稿拼接截 2000 字符；agent/team/dag 直驱；brain 步骤以钉住能力 + 缺省路由
  表纯解析并随 `BrainHandoff` 交接（节点无 brain 运行时也能跑，递归深度按构造
  为 1）；todos 目标本地拒绝（平台语义）。
- 汇总输出按拓扑序 `- {step}: {status}[: {excerpt ≤200}]`。

### Worker

- `project_preflight_agents` 补 `ProjectExecutorKind::Playbook => Vec::new()` 匹配
  臂：步骤在执机时从剧本解析，preflight 不列 agent、不失败（本分支无专属测试）。

## 测试覆盖

| 功能 | 测试函数 | 文件 |
| --- | --- | --- |
| 校验聚合 + diamond 通过 | `validate_collects_every_rejection_with_its_message`、`validate_accepts_the_diamond` | `crates/brain/tests/playbook.rs` |
| 拓扑序/就绪集/失败坍缩/深度宽度 | `topo_order_is_deterministic_across_declaration_orders`、`ready_steps_follow_the_done_set`、`collapse_blocked_takes_the_transitive_downstream`、`chain_depth_and_max_width_metrics` | 同上 |
| prompt 渲染 + 线格式 roundtrip | `render_prompt_substitutes_only_the_situation_placeholder`、`serde_wire_shapes_roundtrip` | 同上 |
| trigger 纯匹配（manual/阈值/空白/坏行） | `manual_never_fires`、`message_fires_at_or_above_threshold_only`、`match_text_trims_and_rejects_blank_patterns`、`scan_filters_corrupt_and_manual_records` | `crates/brain/src/playbook/trigger.rs` |
| Runtime CRUD（含 update 保留 origin/digest、错误 join） | `runtime_playbook_crud_roundtrip`、`update_playbook_preserves_dynamic_origin_and_digest`、`latest_dynamic_playbook_probe_over_the_store`、`create_playbook_with_invalid_input_joins_every_error` | `crates/brain/tests/playbook_runtime.rs` |
| 动态规划（digest 缓存/空库默认/契约/不可解析/缓存隔离） | `plan_persists_and_roundtrips`、`empty_library_returns_default_act_playbook`、`contract_violation_is_typed_generation_failure`、`unparseable_reply_fails`、`distinct_situations_do_not_share_the_cache` | `crates/brain/tests/planning_playbook.rs` |
| v25 表 + save/get/list/delete + latest-by-digest + created_at | `playbook_save_get_list_delete_roundtrip`、`playbook_upsert_preserves_created_at`、`migration_v24_to_v25_creates_brain_playbooks` | `crates/store/tests/brain_store.rs` |
| Step run 终态语义（Plan/Execute/Step 三态） | `plan_finish_writes_back_planned_and_plan_md`、`execute_finish_owns_the_todo_terminal_status`、`step_finish_never_touches_the_todo` | `crates/store/tests/project_runs_finish.rs` |
| Web CRUD 契约（201/400/404/token） | `playbook_crud_roundtrip`、`playbook_create_rejects_invalid_steps_with_400`、`playbook_unknown_id_is_404`、`playbook_update_rejects_invalid_payload`、`playbook_routes_require_token_when_configured` | `crates/web/tests/web_brain_playbooks.rs` |
| 控制面 dispatch（拓扑批次/幂等重放/绑定解析/404/400） | `dispatch_submits_steps_in_topo_batches`、`dispatch_resolves_brain_target_binding`、`dispatch_unknown_playbook_404`、`dispatch_invalid_request_id_400` | `crates/control/tests/e2e/brain_api/playbooks/dispatch.rs` |
| 触发扫描与 list/get | `message_trigger_playbook_is_manually_dispatchable`、`trigger_scan_reports_matching_message_triggers` | `crates/control/tests/e2e/brain_api/playbooks/trigger.rs` |
| 批次拆分/执行 id 纯函数 | `batches_follow_the_dependency_layers`、`step_ids_are_prefixed_deterministic_and_length_safe` | `crates/control/src/api/brain_playbook_dispatch.rs` |
| 本地执行器成功路径（串行/菱形/brain 交接） | `playbook_runs_serial_chain_in_topo_order`、`playbook_runs_diamond_with_parallel_waves`、`playbook_brain_step_routes_via_handoff_without_runtime` | `crates/project/tests/executor_playbook.rs` |
| 本地执行器失败路径（坍缩/todos 拒绝/缺失引用） | `playbook_step_failure_collapses_downstream`、`playbook_todos_target_is_rejected_locally`、`playbook_missing_reference_fails_the_run` | `crates/project/tests/executor_playbook_errors.rs` |

- 全量回归：本轮按操作者指示免测提交（未执行全量 `cargo test --workspace`），
  测试文件已随本提交入库。

## 兼容与范围

- schema v25 迁移幂等（CREATE IF NOT EXISTS）；brain_playbooks 无历史数据需回填。
- Store trait 默认实现对 playbook 方法 bail：mysql/starrocks 等后端明确拒绝；
  libsql 是唯一实现。
- 控制面 dispatch 为一期一次性提交：批次背靠背提交、不跨请求等待步骤完成；
  request_id 幂等只在带键时生效，无键重放会重复建 execution。
- 触发扫描一期仅手动接口，入站消息自动扫描二期；`top_k` 字段已接收未消费。
- 动态剧本规划（`plan_playbook`）当前只有 brain runtime API 与测试调用，无 HTTP
  面；web/control 只暴露固定剧本 CRUD、派发与扫描。
- 本地 project 执行器不跑 `todos` 目标（平台语义，控制面派发）；playbook 的
  brain 步骤要求能力路由表/钉住解析在本地可完成，节点无 brain 运行时仍可执行。

相关语义：[brain](../../../agents/brain/index.md)、[control](../../../agents/control/index.md)、
[project](../../../agents/project/index.md)、[store](../../../agents/store/index.md)、
[web](../../../agents/web/index.md)、[worker](../../../agents/worker/index.md)
