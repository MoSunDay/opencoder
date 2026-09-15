Commit: 5bf6f621e3722e60258592267109ba5807e74d94

# DAG 结果快照与步骤日志抽屉

## 行为

- DAG 运行页与执行详情共用结果画布。进入页面直接显示当前步骤状态，已结束运行不订阅历史事件；运行中从快照水位之后订阅增量，重连时重新同步快照。
- 移除右侧步骤信息、事件列表和底部日志卡片。点击步骤打开右侧 75vw 的「实时日志」抽屉，支持步骤/全部步骤切换、搜索、自动滚动及历史分页。
- 历史日志整批加载，较早内容保留分页入口；关闭抽屉取消加载及实时订阅。日志量不影响画布状态存储，查看运行时暂停后台运行列表轮询。
- 节点通过已有生命周期事件与回执生成 `dag_steps`，返回 `head_seq` 及 running/interrupted 计数；新尝试覆盖旧回执，同次尝试的取消回执不被稍晚的完成事件误判为失败，产物写入错误不被成功回执掩盖；取消后未开始的步骤显示「未执行」。没有数据库迁移或新增配置。

## 验证

- SPA 全量：97 个测试文件、714 项通过（`/tmp/opencoder-dag-spa-final-full.log`）。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告（`/tmp/opencoder-dag-clippy-final2.log`）。
- `cargo test --workspace`：394 个测试套件，5,204 passed / 0 failed / 6 ignored（`/tmp/opencoder-dag-tests-final2.log`）。6 项为原有 NFS 挂载、runc/wasm rootfs 和挂载 CLI 的特权手工用例，没有新增跳过项。
- `cargo build --workspace`：独立工作树通过（`/tmp/opencoder-dag-build-isolated.log`）。共享工作区在测试运行期间加入另一批 host/runtime handoff 改造，其构建因锁跨越 await 失败；本次候选使用已提交基线和明确的 DAG 修复隔离构建，未纳入这些在途改动。
- 浏览器验收入口：`PLATFORM_BIN_DIR=<bundle>/bin node scripts/acceptance/dag_results.js`。独立 Server/Agent 使用确定性模型响应，并校验浏览器实际加载的 SPA 与仓库产物一致。

全量回归发现并修正事件流测试的时序假设：原先固定等待 500ms 后切换节点应答，满负载下可能在首个请求前切成 500；三项相关测试改为实际收到首批事件后推进，断言保持不变。

发布沿用成套 bundle、原子安装、Server → Agent 重启、原 Node ID 校验与 admission reopen 流程；不修改认证数据。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 生命周期快照、游标及大日志隔离 | `snapshot_has_latest_attempts_and_exact_cursor_without_log_payloads` | `crates/store/tests/dag_snapshot.rs` |
| 完成/运行中的进度与单步接口 | `dag_run_progress_and_step_views_after_completion`、`dag_run_progress_reports_running_step_while_in_flight` | `crates/worker/tests/platform/dag_run_steps.rs` |
| 取消回执与产物写入错误 | `cancelled_receipt_survives_its_later_completion_event`、`artifact_failure_after_a_receipt_is_not_hidden` | `crates/worker/src/operations/query/dag_steps/projection.rs` |
| 事件流中途失败、续传与分页 | `mid_stream_node_error_emits_an_error_frame_and_closes`、`incremental_tail_emits_late_rows_and_closes_on_finished`、`more_flag_keeps_polling_and_pages_without_duplicates` | `crates/control/tests/e2e/executions_streams.rs` |
| 快照衔接与结束回执竞态 | `does not replay running when run_finished precedes the journal update` | `crates/web/spa/src/dag/run/progress.dom.test.jsx` |
| 两个入口、75vw 抽屉、步骤切换及取消结果 | `shows final states immediately and loads logs only in the 75vw right drawer` 等 | `crates/web/spa/src/dag/graph.dom.test.jsx` |
| 历史整批展示、断流清理与滚动 | `loads historical pages atomically before subscribing after their last cursor` 等 | `crates/web/spa/src/ui/executionEvents/logs.dom.test.jsx` |

## 已生效版本验收

- 发布 `8a50a393`，四个二进制的 commit、protocol 9 与 SPA 摘要一致，manifest/SHA256SUMS 通过。
- 使用发布包完成独立 Server/Agent 浏览器验收：结果快照、实时状态、75% 右侧抽屉、步骤切换、实时日志和执行详情全部通过。
- 线上 Server/Agent 正常，原 Node ID 在线、Ready、admission=open；历史 DAG 的 5 个步骤直接显示最终结果，没有画布回放 SSE，日志抽屉读取通过。线上 JS/CSS 与提交产物一致，发布后无新增服务警告或错误。
- 回滚包：`/usr/local/bin/.opencoder-platform-rollbacks/1789454112673175689-a8ccb79b028f`。

## 补充修复候选

- `5bf6f621` 保留取消回执与完成事件的正确关系，并消除事件流测试固定延时造成的竞争。
- 分支：`fix/dag-results-final-20260915`；独立工作树：`/data00/github/opencoder-dag-release`。
- 发布包：`/data00/github/opencoder/dist/opencoder-platform-5bf6f621`。四个二进制、manifest、协议与 SPA 摘要验证通过。
- 使用候选发布包的真实 Server/Agent 浏览器验收通过：`/tmp/opencoder-todo-workbench-CUwbzT/dag-results.json`，覆盖结果快照、实时状态、75% 抽屉、步骤日志切换及执行详情入口。
- **尚未上线此补充修复**：本轮 AGENTS.md 要求其他改动验证后一起发布；共享工作区新增运行时改造尚未纳入本次验收，发布范围已向用户请求确认。当前线上仍是 `8a50a393`，健康检查正常、Ready、admission=open，原节点在线。

## 相关文档

- [平台能力](../../agent-platform/index.md)
- [Web](../../../agents/web/index.md)、[Worker](../../../agents/worker/index.md)、[Store](../../../agents/store/index.md)
