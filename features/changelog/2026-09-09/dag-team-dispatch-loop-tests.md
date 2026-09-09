Commit: (working-tree, 基于 b465f440)

# workflow / team 两条链路的最简 control→节点执行闭环测试

补齐 deploy 拓扑下 workflow（DAG）链路缺失的端到端闭环：保存定义 → `/api/dag/defs/:id/dispatch` → control 解析定义快照并选节点 → WS `NodeOperation::Create` → worker 真实执行 → run 视图 / SSE 事件 / 产物契约。team 链路闭环此前分散在断言各异的用例中，现收拢为同一文件里的最简用例（保存定义 → `/api/executions` → done → 成员会话索引 + worker 侧 team/topic 落盘 + final_summary）。

两条用例完全复用现成 mock 基建：真实 `opencoder_control::build_app` + 真实 `Worker` 经 `fleet::run` WS 接入（`Fleet::new`，已内置 ready 快照等待），LLM 侧 `MockChatClient` 脚本队列，零真实模型、零外部网络。DAG agent 步脚本以 ```json 围栏文本驱动 `extract_output_json_from` 结构化恢复，事件断言覆盖 `run_started` / `step_done` / `run_finished` 真实帧。

新增 `crates/worker/tests/platform/dag_team_loop.rs`（157 行）；`tests/platform/main.rs` 挂载模块。另按 lint gate 修复 `crates/worker/src/operations/launch.rs` 一处既有 `clippy::nonminimal_bool`（语义等价改写）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| DAG 保存定义→dispatch→真实节点执行→done | `dag_saved_definition_dispatch_runs_to_done` | `crates/worker/tests/platform/dag_team_loop.rs` |
| DAG run 视图（spec 快照/dag_id/status） | `dag_saved_definition_dispatch_runs_to_done` | `crates/worker/tests/platform/dag_team_loop.rs` |
| DAG SSE 事件（run_started/step_done/run_finished） | `dag_saved_definition_dispatch_runs_to_done` | `crates/worker/tests/platform/dag_team_loop.rs` |
| DAG 步产物 output.json 契约（围栏 JSON 恢复） | `dag_saved_definition_dispatch_runs_to_done` | `crates/worker/tests/platform/dag_team_loop.rs` |
| team 保存→执行→done + final_summary | `team_dispatch_completes_with_final_summary` | `crates/worker/tests/platform/dag_team_loop.rs` |
| team 成员会话索引 + worker 侧 team.json/topic 落盘 | `team_dispatch_completes_with_final_summary` | `crates/worker/tests/platform/dag_team_loop.rs` |

## 验证结果

- 全量 `cargo test --workspace`：**4960 passed / 0 failed / 5 ignored**；360 个测试目标结果（5 项为既有手动环境用例，本轮无新增 ignore）。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告；`cargo build --workspace`：通过。
- 定向回归：`cargo test -p opencoder-worker --test platform` → 7 passed；`-p opencoder-control`、`-p opencoder-dag-runtime` 全绿。
- 行数 gate：新增文件 157 行（≤400）；无硬编码凭据。
- 注：工作树中 `crates/web/tests/support/*` 存在并行会话的在途修改，未触碰；全量回归在包含该在途状态下通过。
