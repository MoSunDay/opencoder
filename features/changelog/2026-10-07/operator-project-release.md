Commit: d5780396e39fa120cefc27cfcede347994234bea

# Operator 项目协作上线与会话目录修正

## 交付

项目执行引用、多人引导及六张 Codex 员工卡已上线。维护升级将项目 schema 迁移到 33、数据格式迁移到 4；随后通过 Server 发布信号滚动切换到本条目基线。已有执行继续由原 Runtime 持有。

发布过程修正了维护预检的 schema 目标、共享数据备份范围及 Nginx 旧工作进程停止受理的确认。知识库保留原有应用运行资产，并加入固定提交的源码和知识索引。

真实 operator 验收暴露并修正两处会话接入问题：DAG 必须从共享 `/workspace/repos` 创建步骤自己的 worktree；Codex 登录目录须保持节点与容器相同的绝对路径，避免共享会话索引记录仅容器可用的路径。注册入口也须使用已安装 codext 的 SQLite 目录规则。六张卡保留原身份、`regression-test` profile 与 `operator` 模式，资源更新至 v3。

本次只修复已核对原始会话文件的路径索引，没有手工变更鉴权数据。旧数据格式的备份不用于恢复已重新开放写入的格式 4 系统。

## 测试覆盖

| 功能 | 测试或验收 |
| --- | --- |
| 多个步骤共享原绝对登录路径，仅挂载一次 | `codex_steps_share_one_home_at_the_original_node_path` |
| profile、私有配置和挂载权限 | `profile_resolution_validates_guest_binary_and_builds_private_mounts` |
| 六卡经 Agent、DAG、Brain 入口复用 | `six_registered_codex_employees_are_reusable_by_agent_dag_and_brain` |
| 合约事件与项目闭环 | `brain_scheduler_v4`、`brain_closed_loop`、`server_restart` |
| 真实六卡、五仓固定提交、步骤独立 worktree | `operators_live_final.py` |
| TODO 派发幂等、同一原生线程跨发布继续输入 | `project_live_recovered.py`、`verify_final_continuation.py` |
| 三版本隔离切换、回滚、旧容器与 Shell 连续执行 | `smooth_release/main.py` |

- Rust 全量回归：5691 passed、0 failed、7 个既有手工用例 ignored；clippy 零警告，完整构建和格式检查通过。
- 隔离演练 147 次提交无失败，最长受理 2.678 秒、最长调度等待 6.920 秒。
- 最终线上滚动切换 36 次容器执行成功，原 Runtime 进程保持不变；最长受理 4.800 秒，最长执行 5.808 秒，公共入口无失败。
- 最终版本完整观察 900 秒：136 次真实容器执行、4501 次入口检查均通过；最长执行 2.860 秒，最长入口检查间隔 0.476 秒。

证据根：`/data00/workspace/artifacts/operator-project-upgrade-20261007/`；交付入口为 `delivery.json`，原始失败、修复与通过回执分别保留。前期功能与 UI 验收见[实现记录](operator-project-execution-index.md)。

相关索引：[DAG Runtime](../../../agents/dag-runtime/index.md)、[调度平台](../../agent-platform/index.md)、[执行配置](../../../docs/registered-runners.md)。
