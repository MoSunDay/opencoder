Commit: (working-tree, 基于 4efaae89bfb89f1ce1c4287cb101ae755c4d526d)

# 项目创建、Agent 执行与逐次回放

## 背景与行为

项目目标、里程碑和 TODO 的创建链路需要可用；TODO 绑定 Agent 后，每次执行都需要保留独立输入、实际资源版本、输出和过程，并可从提交返回的索引重新加载。复用会话、重新发布 Agent、取消或节点重启都不能改写旧运行的数据。

- 修复空目录中首个项目目标的创建弹窗，以及多个目标下的里程碑归属选择；保留分组 TODO 与 backlog 创建。
- Plan/Execute 支持独立 `run_id`，重复提交沿用已接收回执。每次新执行读取当前定义及 Agent 资源；资源身份一致时续用会话，Agent 或版本改变时创建新会话并携带当前方案与前次结果。
- 在执行前保存输入快照，在 Plan/Agent 运行中归档实际模型请求/响应、消息范围、过程事件和子任务关联。显式登记的交付文件保存不可变副本与 SHA-256。
- Plan/Execute 原子互斥并分配版本；run 与 TODO 终态在同一事务提交。归档或事件落库失败会阻止成功收敛和后续接收。
- 取消优先于工具步骤中的部分 assistant 输出；重复取消保留驱动活跃标记直到归档完成。节点中断的历史先封闭消息范围，再允许显式新运行接续。
- 历史分页保留更早记录；消息游标包含当次第一条输入，并排除后续运行。输入、输出、模型文件和大事件以最多 64 KiB 的块读取。页面显示加载错误及历史留存完整度。
- 自定义 Agent 发布与 NFS 导出采用当前 Server 配置的同一资源目录，并隔离不同 Server 的作用域。

## 影响与兼容边界

范围为项目 goal/milestone/todo 及其 Plan/Agent 执行链路。独立 TODO workflow 使用既有入口；Team/DAG 继续使用各自运行详情与 `output_ref`。旧记录缺少输入或过程边界时明确标为 `incomplete_history`，中断记录显示 partial。

SQLite v21 仅增加 `project_todo_runs.input_snapshot` 与 `trace_manifest` 两个可空字段，不新增表。平台 Node 使用 libsql。MySQL/StarRocks 的字段与升级契约已同步；可选特性完成编译和单元验证，没有连接真实 MySQL/StarRocks 数据库。StarRocks 缺少所需跨表事务，Plan/Execute 写入前明确拒绝，结构 CRUD 和历史查询仍可使用。

提交回执提供运行 ID 与所属 Node。`GET /api/executions/:run-id`、`/messages`、`/events-page`、`/detail-field`、事件载荷与 `/artifact` 都经索引读取原节点数据；节点离线明确报错。`GET /api/project/todos/:id/runs?before_version=...` 提供历史分页。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 项目/里程碑/TODO CRUD | `goals_milestones_todos_crud_roundtrip` | [project_api.rs](../../../crates/control/tests/e2e/project_api.rs) |
| 首个目标与多目标里程碑创建 | `creates the first goal from an empty project catalog`、`creates a milestone under the selected goal when several goals exist` | [project.dom.test.jsx](../../../crates/web/spa/src/project/project.dom.test.jsx) |
| 当前大方案按所属索引读取 | `loads a large current plan through the owning TODO execution index` | [project.dom.test.jsx](../../../crates/web/spa/src/project/project.dom.test.jsx) |
| 重试、输入快照与 Agent 切换 | `retries_pin_input_and_agent_changes_start_new_sessions` | [contracts.rs](../../../crates/project/tests/replay/contracts.rs) |
| Plan/Execute 原子接收 | `plan_and_execute_admission_is_atomic_and_versions_are_unique` | [contracts.rs](../../../crates/project/tests/replay/contracts.rs) |
| 原子终态、回滚与页面预算 | `plan_claim_blocks_execute_and_finalization_commits_todo_and_run_together`、`medium_fields_respect_total_page_budget_without_losing_history` | [project_atomic_claim.rs](../../../crates/store/tests/contracts/project_atomic_claim.rs) |
| 交付文件独立副本 | `registered_artifacts_keep_original_bytes_and_digest` | [contracts.rs](../../../crates/project/tests/replay/contracts.rs) |
| 子任务输入、输出与关联 | `child_inputs_outputs_and_links_are_archived_with_the_parent_attempt` | [contracts.rs](../../../crates/project/tests/replay/contracts.rs) |
| 归档失败阻止成功及接收 | `archive_failure_cannot_report_success_or_accept_more_work` | [contracts.rs](../../../crates/project/tests/replay/contracts.rs) |
| 取消时保留部分输出 | `cancellation_after_a_tool_step_retains_partial_output_without_marking_done` | [contracts.rs](../../../crates/project/tests/replay/contracts.rs) |
| 重复取消与收尾互斥 | `repeated_cancel_does_not_converge_a_driver_still_flushing_output` | [service_tests.rs](../../../crates/project/src/service_tests.rs) |
| 压缩后识别本次输出 | `output_identity_survives_compaction_of_prior_messages` | [plan_gen.rs](../../../crates/project/src/plan_gen.rs) |
| 事件背压和数据库错误传播 | `checked_flusher_reports_delta_backpressure_and_database_failure` | [event_sink.rs](../../../crates/session/src/event_sink.rs) |
| 历史分页、大字段、归属、离线读取 | `project_indexes_replay_all_attempts_with_pagination_and_large_fields` | [project_replay.rs](../../../crates/worker/tests/project_replay.rs) |
| 失败尝试保留唯一输入；拒绝后可再 Plan | `failed_attempt_keeps_its_only_input_message_after_later_runs`、`a_rejected_first_execute_does_not_prevent_later_planning` | [project_replay.rs](../../../crates/worker/tests/project_replay.rs) |
| v20 备份、副本升级及重开 | `v20_backup_preserves_history_and_v21_replay_survives_reopen` | [project_replay.rs](../../../crates/store/tests/store_migrations/project_replay.rs) |
| Agent 发布目录与 Server 隔离 | `custom_agent_publication_uses_configured_root_without_cross_server_leaks` | [resource_root.rs](../../../crates/control/tests/resource_root.rs) |
| 浏览器提交重试、历史保留、迟到响应和过程分页 | `keeps the same run identity after a lost receipt and suppresses concurrent submissions`、`loads older history without losing it during refresh`、`ignores a late response from the previously selected TODO and exposes refresh errors`、`opens the exact session and reads oversized input through the run index`、`pages through all archived events using stable sequence cursors` | [replay.dom.test.jsx](../../../crates/web/spa/src/project/replay/replay.dom.test.jsx) |

最终完整回归结果（原始日志目录：`/var/tmp/opencoder-project-validation/evidence/`）：

- `cargo clippy --offline --workspace --all-targets -j 8 -- -D warnings`：零警告，`clippy-liveness-final.log`。
- `cargo test --offline --workspace -j 8 --no-fail-fast`（`RUST_TEST_THREADS=1`）：**4817 passed / 0 failed / 5 ignored**，335 个 suite 结果，`workspace-final-verified.log`。
- `cargo build --offline --workspace -j 8`：成功，`build-final-verified.log`；`cargo fmt --all -- --check` 和改动行数/空白检查通过。
- SPA：**53 个文件、468 项测试通过**，`spa-verified-final.log`；构建通过，`spa-build-verified.log`（既有单包体积提示仍存在）。
- `cargo test --offline -p opencoder-store --features mysql,starrocks --lib -j 4`：**46 passed / 0 failed**，`sql-features-verified.log`；不代表外部数据库集成验证。

5 项既有手动用例分别为 NFS mount、Node NFS/offline，以及 3 项需要 runc/rootfs 的用例；未增加 ignore。NFS 挂载、资源版本与节点离线恢复另由下述双节点验收覆盖。较早一轮全量回归出现会话创建 504；保留原断言单独复现通过，再次完整回归的上述 4817 项全部通过，日志保留为 `workspace-liveness-final.log` 和 `mode-switch-timeout-recheck.log`。

## 隔离运行验收

入口与前提见 [验收脚本说明](../../../scripts/acceptance/project/README.md)。使用构建后的 Server、两个真实 Node、只读 NFSv3、HTTP 模型夹具及 Chromium；没有调用外部真实 LLM。

- **34 次运行**：同一 TODO 31 次、backlog 3 次；31 次成功、1 次模型失败、2 次取消。
- 覆盖稳定 ID 重试、超过 64 KiB 的输入/输出、最早版本浏览器回读、Agent 名称及资源版本变化、交付文件修改后的旧副本下载、Node 宕机/恢复，以及有部分输出后的取消。成功运行终态索引在 10 秒内收敛。
- 观察前后通过索引完整读取并核对 **69 条消息、236 条事件、94 个模型文件**，包含大事件分块；输入、方案、输出、过程清单及消息与只读夹具数据库逐字节一致。
- **30 分钟、181 次采样**，36 个项目索引无重复或丢失，历史摘要和过程清单不变，浏览器错误 0。结束后的完整复核也通过；包含该复核的计时为 `1859745 ms`。
- 两份已登记交付文件再次经索引下载，与各自不可变副本及 SHA-256 一致。验收进程退出码 0，自身服务和 NFS 挂载已清理，证据与测试数据保留。

证据目录为 `/var/tmp/opv/opencoder-project-acceptance-PIp6bZ/`：`report.json`、`observation.json`、`payload-audit.json`、`storage-audit.json`、`oldest-browser.json`、`project-replay.png`、`artifact-final.json` 和 `build-evidence.json`。运行日志为 `acceptance15.log`，汇总回归证据为 `test-results.json`。以上数据来自隔离验收。

## 生产发布与收尾

- 2026-09-08 18:34（北京时间）已将本功能与执行详情抽屉、角色头像一起发布。线上 Server、Agent、CLI 均为 `ff43bfa9410769695481374ea3bd2c5d7e08867c`，protocol 4，三二进制及线上 SPA 与同一 manifest 的摘要一致。
- 仓库随后重写提交信息；发布提交在当前历史中对应 `3564a03e46d139aa892a75734f401118b4dc50ed`。已依据仓库 commit-map 和文件内容核对，运行代码及发布工具未变。后续提交补充验收脚本、逻辑文档和发布记录。
- 发布已完成冻结、执行收敛、一致性备份、同代切换与恢复调度。目标节点 `human-os-02` 的 ID 未变，资源仍为只读 NFS，节点 Ready。38 个执行索引无重复，发布前的 16 个索引全部保留。
- 真实模型的 Agent、DAG、Team、项目 Plan/Execute、大脑稳定 request_id 与 interrupt 链路通过。再次经原运行索引完整读取两次项目运行的输入、输出、4 条消息、562 条事件和 4 个模型文件；归档文件内容一致。既有 DAG 产物重新下载后的 SHA-256 未变。
- Chromium 在线打开最早项目运行，读取输入、输出、事件及模型请求，全部请求成功，浏览器错误 0。
- **两小时生产观察通过**：18:35:24 至 20:35:28（北京时间），`7204.06` 秒、241 次采样；Server/Agent 无意外重启，目标节点持续 Ready，无超时未确认的 Pending 或未收敛的 interrupt。没有认证、协议、存储、进程清理错误或 ERROR/FATAL 日志。三条既有 WARN 来自最初团队验收提示词的协调 JSON 格式冲突，修正该提示后已重新验收通过。

发布包、一致性备份及真实模型验收证据位于 `/var/tmp/opencoder-release-20260908-ui/`。最终复核位于 `/var/tmp/opencoder-release-20260908-project/`：`release-complete.json`、`online-verification.json`、`existing-execution-audit.json`、`published-project-replay.json/png` 和 `production-observation.json`。本次收尾未删除生产数据，也未修改鉴权数据。

## 相关逻辑

[project](../../../agents/project/index.md)、[worker](../../../agents/worker/index.md)、[control](../../../agents/control/index.md)、[store](../../../agents/store/index.md)、[session](../../../agents/session/index.md)、[web](../../../agents/web/index.md)、[平台能力](../../agent-platform/index.md)。
