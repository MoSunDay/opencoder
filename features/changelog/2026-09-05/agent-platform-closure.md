Commit: (working-tree, 基于 c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6)

# Agent 平台：Plan、资源接收与 Python 生命周期闭环

日期：2026-09-05。基线为同日平台实现：4461 项自动测试通过；本轮继续完成 `$do-and-done` 暴露的问题，未执行生产发布。

## 已闭环

| 优先级 | 问题与根因 | 修复及相邻边界 |
| --- | --- | --- |
| P1 | 通用 Plan 没有解析新草稿；节点先保存快照后才检查运行状态 | 统一 Server 控制入口；接收锁内验证后持久化。覆盖原节点归属、伪造快照、类型/目标错误、409/429/预检拒绝零写入 |
| P1 | Python 超时丢弃 blocking 句柄，后台 VM 仍运行；runc 无外部取消 | 内嵌 VM 放入现有 Agent 的内部子进程，先停进程并回收再完成；runc 有界清理和 launcher 回收，长组合 ID 可运行 |
| P1 | 共享 rootfs 的设备初始化导致并发 runc 在 `/dev/ptmx` 上竞争 | 每 bundle 独立 rootfs 副本，复用至重试；重建空设备目录，保留并发执行并用真实容器并行验收 |
| P1 | 终态上报两次失败只写日志，调用方仍正常返回 | 短退避重试后显式返回投递错误，以持续 503 验证，不以静默失败收尾 |
| P2 | 接收前资源失败留下 staging/快照，重试可能继续使用旧资源 | 清理未接收副本；已接收快照保留，NFS 离线仍可继续会话 |
| P2 | 已接收 ID 在 session 建立窗口查询事件返回 404 | 返回合法的空事件流和运行状态 |
| P2 | 完整 NFS/runc、项目控制与窄屏证据不足 | 真实内核挂载、真实容器、双节点进程和打包 SPA 验收；完善发布/观测/回滚说明 |

## 功能 → 测试映射

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| Plan 拒绝无副作用、预检快照清理、已接收空事件流 | `rejected_project_commands_never_change_durable_admission` | `crates/worker/src/operations/admission_tests.rs` |
| 两个入口使用最新草稿、保持同一 Node | `project_plan_act_and_new_draft_stay_on_the_assigned_node` | `crates/worker/tests/platform/workloads.rs` |
| VM 无限循环取消与超时后无存活进程 | `cancellation_kills_and_reaps_an_infinite_vm`、`timeout_kills_and_reaps_an_infinite_vm` | `crates/dag-runtime/src/exec/python/tests.rs` |
| 工作流取消后释放节点容量、保存取消产物 | `dag_cancel_reaps_python_before_releasing_node_capacity` | `crates/worker/tests/workloads.rs` |
| 投递失败回传 owner | `status_delivery_failure_is_returned_to_the_execution_owner`、`invalid_spec_snapshot_fails_before_scheduling` | `crates/dag-runtime/tests/run_loop.rs` |
| NFS 实挂载、只读拒写 | `manual_mount_e2e` | `crates/agents/src/serve/tests.rs` |
| 资源版本隔离、卸载后继续、拒绝新任务 | `readonly_nfs_node_snapshots_and_offline_followup` | `crates/worker/tests/nfs_mount.rs` |
| 复制失败清理 staging | `missing_referenced_resources_fail_preflight` | `crates/worker/tests/resource_snapshot.rs` |
| 解释器版本、设备目录隔离和复制失败清理 | `snapshots_isolate_images_devices_and_retries`、`invalid_mount_parent_does_not_publish_or_leave_staging` | `crates/dag-runtime/src/sandbox/rootfs.rs` |
| 真实 runc 并发执行、长 ID、无限循环取消及超时 | `runc_step_smoke`、`cancellation_and_timeout_remove_running_containers` | `crates/dag-runtime/src/sandbox/runc.rs` |
| 项目最新 Plan、空态、窄屏、崩溃后 interrupted、节点离线错误 | `platform.js` | `scripts/acceptance/platform.js` |

## 验证结果

- `cargo clippy --workspace --all-targets -- -D warnings`：通过，零警告。
- `cargo test --workspace`：312 组，**4467 passed / 0 failed / 4 ignored**。4 项依赖宿主权限的 manual 测试另行全部执行通过，runc 并发场景连续 3 轮通过，另补跑 OCI 专项；原 runc 冒烟不再以缺少 rootfs 时直接 return 冒充成功。
- TUI 搜索用例曾在测试进程继承空管道时失败：`rg` 无显式搜索路径会改读 stdin。对照实验确认相同二进制在标准 headless stdin（`/dev/null`）下搜索正常；最终全量命令显式使用 `/dev/null`，保留原失败日志 `/tmp/opencoder-closure-workspace-pipe-stdin-failed.log`，未修改 TUI 行为或测试断言。
- `cargo build --workspace`：通过。命令使用 `--offline -j 8`，隔离构建目录 `/data00/rust-build/cargo/opencoder-platform`。
- SPA：35 组、335 项通过；构建成功，dist 无漂移。现有单 bundle 大小提示为非阻断构建提示。
- 真实浏览器：PASS，双 Node，1600px/390px、错误场景和 VM 随节点崩溃终止；证据 `/tmp/opencoder-platform-browser-HSOHXg`。
- 行数和新增目录容量检查通过；无新增凭据或业务环境变量。旧数据库和生产环境未变更。

原始输出：`/tmp/opencoder-closure-clippy.log`、`/tmp/opencoder-closure-workspace-tests.log`、`/tmp/opencoder-closure-build.log`、`/tmp/opencoder-closure-manual.log`、`/tmp/opencoder-closure-spa-tests.log`、`/tmp/opencoder-closure-spa-drift.log`、`/tmp/opencoder-closure-browser.log`。

当前影响面没有已知代码阻塞项，达到本地交付验收标准。双节点和 NFS 验收在同一宿主完成，LLM 使用回环夹具；正式部署仍须按 [部署说明](../../../docs/agent-platform.md) 验证目标跨机器网络、NFS 挂载及实际模型配置，并保留配套备份。没有执行生产部署或 Git 提交。
