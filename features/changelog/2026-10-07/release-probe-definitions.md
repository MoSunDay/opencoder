Commit: d36b429017d77f79b4f95741645d8f8e8cd1ef79

# 发布验收持续使用原生探针定义

真实发布验收把固定的 DAG 探针定义传给发布后观察；信号回滚与再次发布分别生成任务 ID，保持探针定义不变。信号失败演练显式要求配套 rootfs，交给隔离环境验证。

## 测试覆盖

| 功能 | 测试 | 文件 |
| --- | --- | --- |
| 观察阶段提交固定原生定义并保存记录 | `test_observation_submits_the_frozen_native_definition` | `scripts/acceptance/smooth_release/tests/test_live_probes.py` |
| 受理不可用时不写成功采样 | `test_observation_rejects_unavailable_admission_without_a_success_sample` | 同上 |
| 回滚、再发布使用不同 ID 和同一探针定义 | `test_signal_roundtrip_keeps_definitions_separate_from_submission_ids` | 同上 |

`python3 -B -m unittest discover -s scripts/acceptance/smooth_release/tests -v`：15 项通过。

## 发布验证

- 代码以 `MoSunDay <MoSunDay@users.noreply.github.com>` 提交并推送到 `main`；正式发布包对应 `d36b429017d77f79b4f95741645d8f8e8cd1ef79`。
- 格式、全工作区构建和全目标 Clippy 通过；`cargo test --workspace` 共 5,677 项通过、零失败。7 项环境用例由 6 项原生测试和真实 Brain 浏览器验收覆盖。SPA 971 项、Computer-use 63 项及全站 15 组 UI/TUI 验收通过。
- 全量 Rust 回归使用 12 个测试线程、65,536 的文件数限额和不含调试信息的测试构建；功能临时目录与构建写入单独隔离。正式发布演练与生产观察使用真实磁盘，发布包保持不变。
- 隔离演练完成真实 runc、切换、回滚和 900 秒观察。生产首次切换期间最大受理耗时 7.05 秒、最大调度间隔 8.70 秒，公共入口无失败请求。
- 生产验收的等待标记需进入 Runtime 的挂载及根目录视图释放，初次验收的失败日志保留。适配执行环境后完整复验回滚与再发布：最大受理耗时 4.70 秒、最大调度间隔 4.42 秒，664 次入口探测无失败；SSE 恢复约 1.10 秒。旧 Runtime、模型 Shell 和独立资源服务在切换中保持原进程。
- 最终版本为 `rel-d36b429017d77f79b4f95741645d8f8e8cd1ef79`，完成 900 秒生产观察，134 次新任务探测全部成功，两个节点保持就绪。
- [主分支 Windows/macOS CI](https://github.com/MoSunDay/opencoder/actions/runs/37557438824) 通过。[MySQL CI](https://github.com/MoSunDay/opencoder/actions/runs/37557438825) 与 [Brain CI](https://github.com/MoSunDay/opencoder/actions/runs/37557438830) 失败；这两条工作流在此前主分支 `35a3fd06` 上也失败，本次未修复。MySQL 报 prepared statement 协议错误 1295；Brain 的完整远端失败日志未能读取，不推断原因。

完整日志与回执保存在 `/var/tmp/opencoder-submit-release-20261007/`，汇总为 `delivery-result.json`，最终生产验收为 `release-live-59dc607bcba6b6c7/result.json`。运行环境适配器保存在同一目录的 `live-with-runtime-mounts.py`。

相关：[发布说明](../../../docs/smooth-release.md) · [Server 模块](../../../agents/server/index.md) · [配置覆盖与安装保护](../2026-10-04/readiness-closure.md)
