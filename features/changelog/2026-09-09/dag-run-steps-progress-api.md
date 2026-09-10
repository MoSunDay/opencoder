Commit: (working-tree, 基于 b465f440)

# DAG run 步级查询：progress 聚合与单步视图 API

补齐控制面只读查询在「DAG run 步级状态」上的缺口：此前 `/api/dag/runs/:id` 只有 run 级 `status`，步级进度要靠 SSE 事件回放拼凑。本轮新增两条路由，数据源直读节点侧每步产物真源 `<dag_root>/<run_id>/<step>/meta.json`（`{"name"/"outcome"/"started_at_ms"/"finished_at_ms"/"error"?}`，outcome ∈ done|error|cancelled）与 `output.json`（总有写入，无结构化输出时为 `null`）；无 `meta.json` 即 `pending`——与 `dag-runtime` 的 `checkpoint::restore` 恢复语义同源，读写两侧共享同一磁盘契约。

- `GET /api/dag/runs/:id/progress` → `{"run_id","execution_status","total","done","error","cancelled","pending","steps":[{"name","status","error"}]}`；`steps` 按 spec 声明序，计数由各步状态折叠，`execution_status` 取 journal 记录的 index 状态串。
- `GET /api/dag/runs/:id/steps/:step` → `{"run_id","execution_status","name","status","error","started_at_ms","finished_at_ms","output"}`；`output` 为解析后的 `output.json`，经 128 KiB 上限的既有 `bounded_value_ref` 收敛（超限折叠为 `read_via:"detail_field"` 占位）。spec 外步名 → 404 `step not found in run spec`；spec 内未跑步 → 200 pending + 空计时 + `output:null`；非 DAG 执行 id → 400 `dag steps require a DAG execution`；未知执行 id → 404（沿用 `for_id`）。

实现沿 `team_turns` 的既有接缝：`PROTOCOL_VERSION` 7→8（hub 注册严格相等校验，加性操作变更沿用 TeamTurns 先例升代际）+ `NodeOperation::DagSteps { execution, step: Option<String> }`；worker 侧新文件 `crates/worker/src/operations/query/dag_steps.rs`（status 折叠/计数/spec 序提取均为纯函数并带单测，meta/output 读取对 IO 与解析失败一律按 pending/Null 收敛），root 选择沿用 `runner::views` 的 legacy 感知（`uses_legacy` → `checked_legacy_workflow_root()`，否则 `kind_root(Dag)`）；control 侧 `compat/workflows.rs` 两个 handler 直通 `executions::for_id`。e2e 覆盖完成态与在途态：完成态两步顺序/计数/单步 `output` 精确断言 + 未知步 404；在途态用 `queue_hang` 把第二步 LLM 调用停在场内，轮询至 `done==1 && pending==1` 断言 `execution_status=="running"` 与第二步 pending/空计时，再经 `/cancel` 收束（fold 为 cancelled）让 fleet 干净关闭。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 完成 run 的 progress 聚合（计数/顺序/execution_status） | `dag_run_progress_and_step_views_after_completion` | `crates/worker/tests/platform/dag_run_steps.rs` |
| 单步视图：status/output（围栏 JSON 恢复值）/i64 计时 | `dag_run_progress_and_step_views_after_completion` | `crates/worker/tests/platform/dag_run_steps.rs` |
| spec 外步名 → 404 | `dag_run_progress_and_step_views_after_completion` | `crates/worker/tests/platform/dag_run_steps.rs` |
| 在途 run：queue_hang 停第二步 → done=1/pending=1、running | `dag_run_progress_reports_pending_step_while_in_flight` | `crates/worker/tests/platform/dag_run_steps.rs` |
| 未跑步单步视图：pending/null 计时/null output | `dag_run_progress_reports_pending_step_while_in_flight` | `crates/worker/tests/platform/dag_run_steps.rs` |
| 在途 run cancel 收束（cancelled 后 fleet 干净关闭） | `dag_run_progress_reports_pending_step_while_in_flight` | `crates/worker/tests/platform/dag_run_steps.rs` |
| 纯函数单测：outcome→status 折叠、计数 fold、spec 序提取 | `operations::query::dag_steps::tests::*`（3 例） | `crates/worker/src/operations/query/dag_steps.rs` |

## 验证结果

- `cargo test -p opencode-worker --test platform`：**12 passed / 0 failed**（含本轮新增 2 例与并行会话 dag/team 用例）。
- worker 新增单测：`cargo test -p opencode-worker --lib dag_steps` → 3 passed。
- `cargo test -p opencode-core -p opencode-control`：**609 passed / 0 failed**（含 e2e mock 节点适配新枚举变体后的全量 control e2e）。
- `cargo clippy -p opencode-core -p opencode-worker -p opencode-control --all-targets -- -D warnings`：零警告；`cargo build --workspace` 通过。
- 全量 `cargo test --workspace`：**361 个测试目标 4998 passed / 0 failed / 5 ignored**（5 项为既有手动环境用例，本轮无新增 ignore）。
- 行数 gate：新增 `dag_steps.rs` 200 行、`dag_run_steps.rs` 198 行（均 ≤400）；改动文件未超限；无硬编码凭据。

## 附注（与计划的偏差）

- run id 采用 `dag-steps-prog-run` / `dag-steps-mid-run`（计划稿的 `steps-prog-run` 不满足 `CreateExecution::validate` 的 `dag-` 前缀约束，会 400）。
- 步依赖字段为 `depends_on`（`crates/dag/src/spec.rs::StepSpec`），计划稿中的 `after` 不存在。
- `crates/control/tests/e2e/support/node.rs` 的 mock 节点对 `NodeOperation` 逐变体穷举匹配，新增变体必须补一条兜底 arm（`DagSteps → miss404`）方可编译——该文件不在计划清单内，属最小必要触碰。
