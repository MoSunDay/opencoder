Commit: e50ffc433bca866fd17bd571a74f1bdf17705dea

# 大脑调度：版本化本体计划与执行工作台

## 问题与行为

此前大脑入口主要围绕需求路由和能力目标绑定，缺少可固定引用、解释输入来源和步骤关系、贯通原生执行过程的计划运行对象。本次增加能力库、计划库和运行工作台：固定版本直接执行；动态模式参考能力和已有版本，一次形成完整本体计划，发布后自动执行。

计划版本不可覆盖，保留 changelog、标签、置信依据和人工稳定标记。运行依赖按类型化端口解析，无关步骤并行，完成一步即唤醒其后继。暂停、取消、缺失输入和共享资源等待都有持久状态。Web 画布复用四类执行的查询/渲染组件，展示实际输入输出及可下载证据。

## 实现与边界

- 新增 Fleet Brain 类型（协议 9），根节点以 TODO 存储状态。短 runc 激活挂载 CLI；等待时释放执行槽。控制面维持原五字段索引。
- 计划定义、agent 资源摘要和 harness 配置固定；派发前选择匹配节点并在受理时复核。Prepared 动作先落盘，未知受理结果保持同 ID，通知提交后确认，重放去重，分批轮转。
- 同名资源跨运行读/写互斥，明确结束回执才释放，不按时间过期。计划与资源事务共享连接锁；其他定义写入不能混进事务后被回滚。
- 一次运行固定完整计划，不做运行中重规划。根节点支持原盘重启恢复，不包含跨节点丢盘接管。
- 修正构建时 Git 元数据路径解析：linked worktree 使用真实 HEAD/refs 路径，避免监视不存在路径造成每次全量重编。
- 恢复 TODO 管理中模板环境与工具入口，保留同期 Env 配置管理行为；原生 DAG 和嵌入过程共用画布。
- 隔离平台与 Brain 集成测试的配置目录，避免读取宿主机凭证、harness 配置和 NFS 资源池。定位到原平台测试并发复制宿主资源耗时接近 15 秒并触发 RPC 超时；保留生产受理与超时契约。

## 测试覆盖

| 功能 | 关键测试 | 文件 |
| --- | --- | --- |
| 本体类型、绑定、循环 | `rejects_cycles_unknown_ports_wrong_types_and_invalid_semantics` | `crates/brain/tests/ontology.rs` |
| 非屏障并发 | `successor_starts_while_an_independent_sibling_is_running` | `crates/brain/tests/execution.rs` |
| 持久输入、控制与验收 | `plan_input_is_persistent_validated_and_immutable_after_supply`、`pause_fences_old_decisions_and_cancel_waits_for_child_receipts`、`output_contract_failure_is_visible_and_independent_work_continues` | `crates/brain/tests/execution.rs` |
| 批量条件与汇合 | `sealed_expansion_skips_false_branches_and_joins_all_outputs` | `crates/brain/tests/execution.rs` |
| 不可变版本与资源互斥 | `versions_are_append_only_conflicts_do_not_move_stable_pointer`、`cross_run_claims_allow_shared_reads_and_require_real_release_before_write` | `crates/store/tests/brain_versions.rs` |
| 同连接事务隔离 | `unrelated_definition_write_cannot_join_a_rolled_back_plan_transaction` | `crates/store/src/fleet/brain.rs` |
| 固定/动态计划与真实节点通道 | `fixed_plan_executes_through_real_node_channels_and_returns_verified_outputs`、`dynamic_plans_once_publishes_version_and_registers_draft_capabilities` | `crates/worker/tests/brain_ontology.rs` |
| 输入等待、释放槽位、暂停取消 | `input_wait_releases_slot_and_pause_fences_then_cancel_closes_root` | `crates/worker/tests/brain_ontology.rs` |
| Team/DAG/TODO 产物 | `managed_team_dag_todo_return_typed_downloadable_outputs` | `crates/worker/tests/brain_outputs.rs` |
| 重放去重与规划前控制 | `prepared_action_replays_after_restart_and_duplicate_notice_keeps_watermark`、`planning_can_pause_before_activation_and_cancel_without_a_plan` | `crates/worker/tests/brain_recovery.rs` |
| 真实容器挂载 CLI | `mounted_cli_finishes_fixed_activation_without_model_credentials` 显式 runc 冒烟 | `crates/worker/src/brain/container.rs` |
| 无效 JSON 阻止保存 | `blocks saving invalid JSON instead of submitting a stale valid binding` | `crates/web/spa/src/brain/workbench/tests/editor.dom.test.jsx` |
| 不确定提交保持运行 ID | `keeps a durable run identity across an uncertain submission and exposes errors` | `crates/web/spa/src/brain/workbench/tests/launch.dom.test.jsx` |
| 共用过程与原入口兼容 | SPA DAG/fleet/app/TODO/Env 回归；Chromium 桌面、窄屏交互 | `crates/web/spa/src/` |

## 验收结果

- `cargo test --workspace --no-fail-fast`：5,049 项通过，零失败；平台 12 个并发用例在完整工作区特性组合下全部通过。
- `cargo clippy --workspace --all-targets -- -D warnings` 与工作区构建通过。
- SPA 全量 652 项测试通过；最终模块调整的针对性回归、构建及 dist 漂移检查通过。
- 挂载 CLI 的真实 runc 冒烟显式通过；Chromium 使用接口夹具验证桌面、深链接、本体画布和窄屏，无页面错误或横向溢出。

产品与协议说明：[功能入口](../../brain/index.md)、[契约文档](../../../docs/brain-orchestration.md)。
