Commit: 8bf74a10109dc16c0d087df23e1ea829ed1dd259

# 仓库范围清理与项目执行结果读取

仓库只保留 OpenCoder 通用基建、可复用工具、测试和说明。业务专用统计脚本、相关历史描述和现场信息移到仓库外；19 份原稿已按摘要核对。README、仓库规则和 AGENTS 明确相同边界，Windows 安装入口改为显式提供主机地址和允许访问的网关。

项目模块按用户确认的方案只保存执行引用，结论通过所属节点实时读取。移除本地尚未提交的结论缓存、后台收集和重新收集实现，保留标签、全屏详情、Ontology、上下文容量检查、历史结果读取与执行容量恢复等通用平台改动。规则 07 与验收脚本同步到实际行为。

验证和提交使用独立工作区；验证开始后的并行前端修改继续保留在原开发工作区。

## 测试覆盖

| 功能 | 测试或检查入口 |
| --- | --- |
| 仓库范围、密钥格式、冲突标记、文件行数及文档链接 | 全部待提交文本与改动文件审查 |
| 四类能力派发、回复丢失重试、实时结果、离线、失败、空结果、取消与手工进度 | [project/capabilities/main.js](../../../scripts/acceptance/project/capabilities/main.js) |
| 注册 Operator 经 Agent、DAG 和 Brain 复用及上下文能力声明 | `six_registered_codex_employees_are_reusable_by_agent_dag_and_brain`：[operators.rs](../../../crates/control/tests/e2e/project_links/operators.rs) |
| 专项与 TODO 全宽详情 | `opens project progress, then the initiative board and TODO in full-width right-side drawers`：[project.dom.test.jsx](../../../crates/web/spa/src/project/project.dom.test.jsx) |
| 旧 Runtime 的历史结果、Host 重启、休眠唤醒与回滚 | `released_runtime_results_survive_host_upgrade_sleep_wake_and_rollback`：[result_upgrade.rs](../../../crates/agent/src/host/tests/result_upgrade.rs) |
| 真实 NFS 与原生容器 | `manual_mount_e2e`、`kernel_plain_readdir_continues_across_exporter_replacement`、`readonly_nfs_node_snapshots_and_offline_followup` 及 runc 的三项手工用例 |
| 流式断流、心跳空闲与重试耗尽；模拟服务完整读取请求体 | `stream_retry` 的五个用例：[stream_retry.rs](../../../crates/llm/tests/stream_retry.rs) |
| 全部页面、四种屏宽、Brain、Ontology 与 TUI | [ui/main.js](../../../scripts/acceptance/ui/main.js) |
| 独立服务的升级、回滚、旧任务连续性与持续观察 | [smooth_release/main.py](../../../scripts/acceptance/smooth_release/main.py) |

## 验证结果

- 后端全量回归：`cargo test --locked --workspace --no-fail-fast -- --test-threads=8` → 455 组，5,716 passed / 0 failed / 8 ignored，进程退出码 0。实际输出保存在下方验证目录的 `workspace-verified.log`。
- 流式重试五个用例连续运行五轮全部通过，并在全量回归中再次通过。
- 格式、全目标 Clippy 与工作区二进制及示例构建通过。
- SPA：150 个文件、1,060 项通过；Ontology 类型检查及独立 97 项测试通过；隔离提交副本的产物漂移检查通过。
- 全站 UI/TUI：16 项全部通过，覆盖全部 15 个导航页及 1920、1280、768、390 四种屏宽。隔离副本的 Ontology 五页与真实只读 NFS 补测通过。
- 7 项 NFS、runc 和旧 Runtime 手工用例通过；默认忽略的 Brain 浏览器用例已包含在全站验收中。
- 发布、维护、备份与清理脚本回归通过。Windows 安装参数按脚本和文档静态核对，未在 Windows 环境执行安装。
- 平滑切换在独立临时存储中完成 900 秒观察，49 次连续提交无失败，观察阶段 57 个任务全部完成。原 30 秒门槛保持不变。本次是开发验证，没有发布生产服务。

首轮 Rust 回归发现旧夹具缺少 Brain 上下文能力声明，补齐后复验。流式重试模拟服务完整读取请求体后再返回响应，避免分段请求留下未读数据；正常响应统一写入，保留原有超时阈值和全部断言。代理排除测试使用文档示例地址。

共享磁盘上的一次观察任务超过 30 秒，DAG 页面启动也曾超时；后续在独立临时存储中验证。Rust 隔离测试使用同一份源码构建配套二进制与容器镜像，避免与旧构建的版本信息混用；源码以摘要清单核对。此前失败日志保留在外部验证目录。

源码和验证清单位于 `/tmp/opencoder-cleanup-validation-20261009/`，原生、全站与切换回执位于 `/data00/opencoder-safety/cleanup-20261009/validation/`；仓库外原稿及其摘要清单位于 `/data00/opencoder-tools/repository-cleanup-20261009/`。

相关：[项目模块约定](../../../rules/07-project-module-contract.md)、[项目工作台](../../project/index.md)、[仓库逻辑地图](../../../repo-memory.md)。
