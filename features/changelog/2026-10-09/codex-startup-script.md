Commit: 1ff961846496dd24f0a32f7e5106bbee0e234a5a

# Codex 外部启动脚本

Harness 管理使用启动脚本入口替代环境变量编辑。默认设置和命名档案保存 `startup_script` 命令数组；每次启动把 Codex 命令逐项追加给脚本，复用既有工作目录、输入输出与进程取消机制。启动参数和非凭据会话上下文由平台传递，账号与资源策略放在外部脚本。

移除专用 `auth_slot` 字段；旧配置即使将它保存为 `null`，也须在切换版本前移除。脚本入口随会话固定，动态环境不写回配置。容器准入检查镜像内的脚本入口，不回退宿主执行。

## 测试覆盖

| 功能 | 测试 | 文件 |
|------|------|------|
| 命令校验及配置覆盖 | `profile_validation_rejects_invalid_execution_settings`、`project_codex_overlay_preserves_global_launch_settings_and_frozen_operator` | [配置校验](../../../crates/core/tests/harness_runtime.rs)、[覆盖](../../../crates/core/tests/config_overlay/main.rs) |
| 脚本设置 PATH/环境、续会话与分叉固定入口 | `external_launcher_sets_environment_and_wraps_each_pinned_resume_and_fork` | [启动脚本](../../../crates/session/tests/harness_startup.rs) |
| 启动失败不回退、取消准备进程树 | `failed_or_missing_startup_script_never_falls_back_to_codex`、`startup_cancellation_reaps_script_and_descendant_before_codex_launch` | [启动脚本](../../../crates/session/tests/harness_startup.rs) |
| 节点排队和继续会话固定脚本设置 | `managed_codex_is_pinned_and_node_obeys_fifo_lifo`、`idle_codex_followup_waits_for_capacity_and_keeps_its_settings` | [节点配置](../../../crates/worker/tests/harness_settings_queue.rs) |
| 镜像内脚本入口校验 | `profile_resolution_validates_guest_binary_and_builds_private_mounts` | [容器配置](../../../crates/dag-runtime/src/sandbox/codex/tests.rs) |
| 创建、保存、失败重试和四种屏宽 | Harness 组件测试与浏览器验收 | [组件](../../../crates/web/spa/src/harness/management.dom.test.jsx)、[浏览器](../../../scripts/acceptance/harness/startup.js) |
| 真实容器脚本执行及上下文 | `server_dispatches_codex_in_runc_with_node_login_profiles_and_cancellation` | [容器回归](../../../crates/worker/tests/dag_codex_runc.rs) |

按用户要求执行本次范围的回归，配置、会话、节点、容器配置单元测试和前端测试通过；SPA 独立目录构建与浏览器检查通过。

独立 Server / Node 注册 Operator 后完成三轮真实 Codex 调用，五项验收全部通过：首次执行外部脚本、原会话固定脚本入口、新会话采用更新配置、脚本失败不回退、取消准备阶段清理进程树。模型实际读取脚本生成的环境变量和工作目录文件；外部脚本负责账号申请、注入与释放，账号占用和临时凭据均已清理。详细验证计划、脚本和执行回执保存在仓库外。

综合容器回归在多 Operator 步骤持续待执行处超时；独立容器诊断阻塞于内核挂载锁，真实容器验收未完成，不作为发布通过结论。未执行全仓回归或线上发布。

实现索引：[core](../../../agents/core/index.md)、[session](../../../agents/session/index.md)、[web](../../../agents/web/index.md)、[dag-runtime](../../../agents/dag-runtime/index.md)。使用说明见 [Codex 启动脚本](../../../docs/codex-startup-script.md)。
