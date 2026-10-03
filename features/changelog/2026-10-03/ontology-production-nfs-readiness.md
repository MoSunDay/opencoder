Commit: 9d82393d5ad376511b387d089199a4d845f22b08

# Ontology 上线与维护升级的 NFS 读取等待

## 背景与变化

资源服务已恢复 HTTP 和 NFS 监听后，已有内核 NFS 客户端仍可能暂时报 `EIO`。维护安装阶段立即读取目录会中断升级，公共入口继续返回 503。

安装阶段在发布等待时限内重试真实资源检查，实际目录可读后才准备候选进程。超时保留关闭的写入口、阶段和已验证备份；同一候选续跑复用原备份与固定配置，不重新制作备份。

## 验证映射

| 行为 | 测试 | 位置 |
|------|------|------|
| NFS 临时读取失败恢复后才准备候选进程 | `test_kernel_nfs_recovery_finishes_before_candidate_processes_start` | [test_flow.py](../../../scripts/platform/maintenance_tests/test_flow.py) |
| 超时保持写入口关闭，同候选续跑使用未改变的备份 | `test_nfs_timeout_preserves_closed_gate_and_resumes_the_same_backup` | [test_flow.py](../../../scripts/platform/maintenance_tests/test_flow.py) |

- 发布控制器测试：维护 51、滚动发布 62、信号发布 12，共 125 项通过。
- 正式候选 `399161f0284d0357045e59ad54cf96b7870e923e`：Clippy、构建及全量 Rust 回归通过，5,646 passed、0 failed。正式包六个二进制及内置 SPA 的构建信息和摘要一致。
- 正式包全站 UI 验收 15 项通过，覆盖 1920、1280、768、390 四种宽度；线上另验证五个 Ontology 页面中的真实数据、G6 图谱及 240 像素下完整标签与横向滚动。
- 生产从已封存备份完成数据格式 3 的维护升级，公共入口恢复；实际运行的 Server 二进制与正式包摘要一致。Ontology 实体、关系与幂等重试通过，真实 NFS 挂载可读取并核验正文，写入被拒绝且未导出数据库。临时验收环境停用后，正文 NFS 恢复默认关闭。
- 最终版本与配置固定后完成 902.37 秒生产观察，29 次新的原生任务全部终态完成，健康、节点、NFS 与版本检查无失败。旧格式备份恢复保护验证为零副作用拒绝。

原始回执、构建摘要、截图及观察记录保存在 `/root/.cache/opencoder-e2e/20261003-ontology-closure/`；凭据和临时数据库不纳入 Git。

## 相关索引

- [Server](../../../agents/server/index.md)、[维护升级](../../agent-platform/index.md)
- [Ontology](../../ontology/index.md)、[发布说明](../../../docs/smooth-release.md)
