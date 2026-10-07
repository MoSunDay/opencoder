Commit: c854143bd187656f4d74be6cca0f153176e44a21

# Operator 项目执行引用与多人引导

## 变更

项目 TODO 选择真实能力并关联原生执行，同一 TODO 可由多个 operator 分别执行，多人也可打开同一个执行 ID 共享上下文和输入引导。TODO 不建立独占领取锁、额外调度循环或自己的 Codex 线程；看板完成由人维护。

- `POST /api/project/todos/:id/dispatch` 复用 Fleet 请求回执与 Agent、Operator、DAG、Team、TODO 工作流、保存的 Brain 计划入口；稳定执行 ID 固定能力与请求，重试拒绝不同输入。执行已受理而关联失败时显式返回 `accepted=true, linked=false`。
- 关联只保留能力与执行引用。`GET /api/executions/:id/result` 向所属节点读取当前结论，离线返回错误；DAG 完整结果仍由步骤明细提供。删除 TODO 不删除执行。
- Web 从能力库选择具体能力，CLI 增加 `project todos links/attach/detach/dispatch`、`exec result`。人工引导在回复丢失时沿用原输入 ID。
- 六张既有 Codex operator 卡保持名称、Harness、profile 和运行模式；覆盖独立 Agent、DAG Agent 步骤及 Brain 能力引用。Brain 的创建、整层终态和人工输入事件合同保持不变，同层能力并行执行。
- 注册资源中的路径和 worktree 帮助脚本随 skill 包分发；DAG 使用已有可写步骤仓库并校验固定基线，不复制整个源仓库。配套源码位于工作区的 `tools/operator-routing/` 与六个 `.agents/skills/*`。
- 发布镜像副本保留内部硬链接，但不与源镜像共享可写 inode，两个运行器仍独立固定并校验。

## 数据与发布

项目 schema 升到 33，事务迁移移除执行结论和同步状态缓存，保留关联 ID、能力信息和手工看板字段；没有新增表或运行环境变量。数据格式升到 4，格式 1–3 到 4 必须维护升级，禁止旧 Server 打开新库或混合滚动。维护完成后可在格式 4 版本间平滑切换和回滚。

本交付是基于上述提交的未提交工作树与开发构建；未执行生产部署、重新注册线上资源或变更已有鉴权数据。正式发布仍需从审查后的提交构建严格发布包。

## 测试覆盖

| 功能 | 测试名或验收入口 | 文件 |
| --- | --- | --- |
| 六张员工卡经三种入口复用，共 18 次受理 | `six_registered_codex_employees_are_reusable_by_agent_dag_and_brain` | [operators.rs](../../../crates/control/tests/e2e/project_links/operators.rs) |
| 重试固定请求和能力、拒绝不同输入 | `dispatch_retry_freezes_capability_and_rejects_reuse_with_other_input` | [dispatch.rs](../../../crates/control/tests/e2e/project_links/dispatch.rs) |
| 同 TODO 多执行，无独占领取 | `separate_operators_can_work_on_the_same_todo_without_an_assignee_lock` | [dispatch.rs](../../../crates/control/tests/e2e/project_links/dispatch.rs) |
| 读取节点当前结果、离线不返回缓存成功 | `results_are_read_from_the_owner_and_report_node_failures_without_cached_success` | [dispatch.rs](../../../crates/control/tests/e2e/project_links/dispatch.rs) |
| 两个人使用同一执行和各自输入 ID | `shared_execution_guidance_keeps_each_human_input_id_and_the_same_node` | [dispatch.rs](../../../crates/control/tests/e2e/project_links/dispatch.rs) |
| v30/v32 关联迁移与手工字段保留 | `v30_execution_links_upgrade_without_losing_ids`、`v32_migration_removes_execution_caches_and_keeps_index_metadata` | [project_relations.rs](../../../crates/store/tests/project_relations.rs) |
| 六张 Codex 卡在同一真实 runc 容器中并行，固定 profile、工具事件和清理 | `server_dispatches_codex_in_runc_with_node_login_profiles_and_cancellation` | [dag_codex_runc.rs](../../../crates/worker/tests/dag_codex_runc.rs)、[registered.rs](../../../crates/worker/tests/harness/registered.rs) |
| Brain 合同、闭环和重启 | `brain_scheduler_v4`、`brain_closed_loop`、`server_restart` | [worker tests](../../../crates/worker/tests/) |
| 数据格式隔离 | `project_reference_schema_requires_maintenance_from_previous_formats`、`test_project_reference_format_verifies_but_cannot_overlap_cached_result_servers` | [release.rs](../../../crates/core/src/fleet/release.rs)、[test_manifest.py](../../../scripts/platform/rolling_tests/test_manifest.py) |
| 镜像内部硬链接和源副本分离 | `test_frozen_image_preserves_internal_hardlinks_without_linking_the_source` | [test_native.py](../../../scripts/platform/rolling_tests/test_native.py) |
| Web 能力派发重试、结果读取、输入 ID | `launcher.dom.test.jsx`、`result.dom.test.jsx`、`inputAttempt.test.js` | [execute](../../../crates/web/spa/src/project/execute/)、[inputAttempt.test.js](../../../crates/web/spa/src/chat/inputAttempt.test.js) |
| 全站四种宽度、真实交互及 TUI | 15 项全部通过 | [ui/main.js](../../../scripts/acceptance/ui/main.js) |
| 三版切换、回滚、持续任务及 15 分钟观察 | `smooth_release/main.py --observe-seconds 900` | [main.py](../../../scripts/acceptance/smooth_release/main.py) |

## 验证结果与证据

证据根：`/data00/workspace/artifacts/operator-project-index-20261007`。

- `cargo test --workspace --no-fail-fast --offline`：5685 passed / 0 failed / 7 既有手工项 ignored。每个测试可执行文件使用独立网络和挂载命名空间，临时文件放在独立目录；真实 runc 用例实际执行。Brain 浏览器手工项另在 UI 验收执行。原始输出：`rust-v4.log`、`rust-v4-counts.json`。
- `cargo clippy --workspace --all-targets -- -D warnings`、全工作区构建与格式检查通过：`clippy-v4.log`、`build-v4-confirm.log`。成套二进制与最终源码产物摘要核对：`artifact-source-check.json`、`source-stability.json`。
- SPA：134 文件、980 测试通过，见 `spa-regression-final.log`。全站 15 项通过，含 1920/1280/768/390 宽度与 TUI，见 `ui-acceptance-v4/receipt.json`；截图、报告和日志已归档。
- 发布脚本 143 项、operator 资源帮助脚本 5 项通过：`release-scripts-current.log`、`operator-resources.log`。Store 所有 feature 编译通过；未使用真实 MySQL/StarRocks 实例做运行验证。
- 正式数据所在 `/dev/vda2` 的独立演练通过：68 次连续提交无失败，最长受理 0.823 秒、受理间隔 0.795 秒、调度间隔 0.966 秒、调度等待 1.518 秒；完整观察 900 秒，57 个任务全部通过，最长 1.519 秒。证据：`smooth-release-production-disk/result.json`、`release-storage-evidence.json`。
- 较早放在工作区 `/dev/vdb` 的演练观察任务曾耗时 85.937 秒而失败。复测捕获 `ovl_sync_fs → sync_filesystem` 等待底层全盘写回；正式数据库、Runtime 写层和镜像实际位于 `/dev/vda2`，故使用同盘独立目录重新验收，没有降低 30 秒门槛或改用内存存储。失败与等待栈保留在 `smooth-v4.log`、`retest-waits.jsonl`；原始最终演练目录为 `/var/tmp/opencoder-operator-project-index-20261007/opencoder-smooth-tnb67lmw`。
- 候选包 `candidate-native.tar.gz` 已回读校验 7 个应用/运行器及镜像内 3 个运行器摘要，保留 135 个硬链接；见 `archive-verification.json`。测试服务均已停止。

## 相关索引

[项目工作台](../../project/index.md)、[调度平台](../../agent-platform/index.md)、[项目逻辑](../../../agents/project/index.md)、[发布说明](../../../docs/smooth-release.md)。
