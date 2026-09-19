Commit: (working-tree)

# 三个 review 门禁 DAG：seed 激活修复与 R1–R5 e2e 全绿

## 背景

f5907145 交付了三个内置 review 发布门禁 DAG（`review-full-acceptance` /
`review-harness-quick` / `review-code-quick`）的 runtime 扩展（`opencoder`
host imports、`dag.ops` 受控 op 注册表）、wasm 模块 crate
（`crates/dag-review-tools`）与启动 seed，但 R1–R5 e2e 全部失败：seed 在
`GET /api/dag/defs` 不可见、节点报 wasm module not found。本次定位两处根因
并修复，e2e 五用例全绿。

## 变更

### 1. seed 落到控制面真实存储（`crates/control/src/seed_dags.rs`）
- 根因：seed 原写 libsql definitions.db（`state.store`），而控制面
  `/api/dag/defs` 路由（`api/catalog.rs`）读写 `state.fleet`（FleetStore 的
  `fleet_definitions` 表，control.db）。seed 日志 printed `seeded=3` 但列表
  恒空。
- 修复：`seed_review_dags(&Arc<FleetStore>)` 改用 `fleet.definitions("dag")`
  判重、`fleet.put_definition("dag", name, dag_definition(...))` 插入；body
  形态复用 `api::catalog::dag_definition`（id==name + created_at/updated_at
  元数据），与 `POST /api/dag/defs` 完全一致。skip-don't-overwrite 语义、
  warn-never-block 启动门保留；单测改跑 `FleetStore::open_memory`。

### 2. fleet 测试配置深合并（`tests/support/fleet_proc.rs`）
- 根因：`write_config` 的 extra 是顶层键覆盖——`{"dag":{"ops":…}}` 把默认
  `{"dag":{"wasm_dir":…}}` 整节替掉。server 端 publish 走 data-dir 兜底照常
  201，节点端 `dag.wasm_dir=None` → accept 时 wasm pin 静默 no-op →
  `_modules` 永不物化 → 运行时 module not found。
- 修复：对双方均为对象的顶层键做一层深合并（extra 的 `dag.ops` 并入默认
  `dag` 节），杜绝同类复发。

### 3. e2e 修复与拆分（`tests/dag_e2e/review_dags/`）
- 断言路径：`wait_terminal` 返回 run 文档，终态在
  `execution.status`/`dag_steps`；fail-closed 步错误在
  `dag_steps.steps[].error`；GET run URL 带全 `dag-` 前缀。
- stub FIFO：`LlmStub` 的 script 每条目只应答一个请求——R4 四段 agent 链
  传 `vec![router; 8]`（对齐 code_review 的 `;24` 手法），否则第 2 步起拿到
  exhausted 兜底文本、output.json=null。
- 上下文断言按紧凑序列化匹配（注入 JSON 无空格）；五字段投影在
  `/api/executions?kind=dag` 索引行断言，run 文档按实际字段
  （id/name/status/created_at/kind/node_id）。
- 455 行单文件拆为 `review_dags/mod.rs`（R1–R5）+ `review_dags/helpers.rs`
  （probe server、op 脚本、数据段 WAT 构造、publish/dispatch、产物读取）。

## 测试覆盖

| 功能 | 测试 | 文件 |
|------|------|------|
| R1 启动 seed 可见/幂等/spec 有效 | `r1_review_defs_are_seeded_and_visible` | `tests/dag_e2e/review_dags/mod.rs` |
| R2 host imports：noop op（argv+env 透传+证据日志）、HTTP probe、未知 op fail-closed | `r2_host_imports_run_ops_probe_and_fail_closed` | 同上 |
| R3 harness-quick 池模块+seeded def 原样 dispatch（NDJSON 证据+结构化 verdict） | `r3_harness_quick_chain_runs_the_seeded_def` | 同上 |
| R4 code-quick 四段 agent 链：上下文逐级传递+五字段执行索引 | `r4_code_quick_agent_chain_passes_context_downstream` | 同上 |
| R5 full-acceptance 三模块四 op+probe、seeded spec 原样 dispatch | `r5_full_acceptance_dispatches_the_seeded_def` | 同上 |
| seed 三态（首轮插入+round-trip / 幂等 / 操作员编辑存活） | `seed_dags::tests::*`（3 个） | `crates/control/src/seed_dags.rs` |
| write_config 深合并（dag.ops 不顶掉 wasm_dir） | 随 R2/R3/R5 的模块物化 | `tests/support/fleet_proc.rs` |

- `cargo test --test dag_e2e`：14/14 绿（含 R1–R5）。
- `cargo test -p opencoder-control`：257 绿；`cargo clippy --workspace --all-targets`
  仅剩既有 code_review.rs useless_format（已提交代码，非本次引入）。
