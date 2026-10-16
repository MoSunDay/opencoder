Commit: ca56953542ab475a67975dc6e06bf7cfa0db1fbd

# Codex 启动脚本在 DAG 容器内使用同版运行环境

宿主可用的 Codex wrapper 不能自动在 DAG rootfs 使用；容器 PID 写入宿主占用账本会被误判为已退出。managed startup 镜像现在固定 wrapper/Codext 内容摘要，私有配置按运行冻结，连接和 token 文件仅通过确切只读挂载提供。

每次 runc 执行的宿主 broker 复用既有锁与账本，保留旧宿主占用。步骤进程树清理完成才释放；Runtime 异常断开保留到对应容器消失。平台 session/thread/input 不改变，Agent 不回退宿主。

| 功能 | 定向验证 |
|---|---|
| 同版代码、私有文件、冻结配置与缺依赖拒绝 | `managed_startup_freezes_config_and_mounts_only_declared_private_files`、`managed_startup_rejects_drift_aliases_and_repeated_state_root` |
| 同版 CLI 镜像与旧镜像保留 | `test_managed_startup_image_pins_platform_cli_and_does_not_change_legacy_image` |
| 单存储根、旧占用兼容、异常断开不提前释放 | startup `test_state.py`、`test_broker.py` |
| 真实 runc 并发、session 回执、退出码与取消清理 | `codex-startup-probe` example；使用隔离额度及进程 fixture，无真实鉴权数据 |

相关：[运行时结构](../../../agents/dag-runtime/index.md)、[DAG 能力](../../dag/index.md)、[配置](../../../docs/registered-runners.md)。

生产验收（同包 `21d21a6bf3274d74d7d764b6f0b86d001c07bc03`）：

- 79.43 与 71.70 已由 Server 信号发布；保留原 Runtime、任务、NFS 与各节点授权。
- `dag-codex-runc-9f-tools-20261014` 在生产 runc 中完成真实 Codext 工具回合，平台 session/input 与账号回执一致，thread 持久化；声明的外部连接及其确切 token 文件只读可打开，没有读取或输出凭据值。完成后账号占用释放、容器状态清空。
- `dag-codex-runc-9f-cancel-20261014` 取消后 8 个容器进程与宿主 broker 均退出，账号回执 released、退出码 143。公共取消曾返回 Runtime 504，随后同一执行 ID 重放取得 cancelled；首个接口失败记录保留，不能表述为取消接口全程通过。另一个零模型 native 长步骤的同一控制链复验在 3.72 秒取得 cancelling，4.72 秒清理完成；未修改已有任务。
- 71 完整发布验收 PASS，117 个原生观察探针覆盖 900 秒，真实 TODO 依赖链和 SSE 恢复通过。两节点公网首页、health、ready 另有连续 900 秒、各入口 2396 次零失败记录；此前 79 同一轮三次 4 秒超时保留。
- 79 前两次执行观察未通过 30 秒门槛：首次在 20/20 总容量满载时按 FIFO 排队约 76.8 秒，进入容器后正常完成；续观察单步约 177 ms 完成，但约 54.6 秒才清理至终态，同期内核报外部知识库 NFS 不响应。不得提前释放占用、使用脱离挂载掩盖等待，或删除现有任务需要的知识库能力。隔离 runc A/B 验证否定了 `--no-pivot` 本身的假设。NFS 恢复后，在同一发布记录下完成连续 900.006 秒、156 个探针的续观察，最大整次探针耗时 8.38 秒，Runtime 和 NFS 进程身份保持；前两次失败保留。

完整证据保存在发布目录 `runners/release-observations/21d21a6b-{79,71}` 与制品目录 `codex-runc-startup-20261014/21d21a6b`；失败与续验收各自保留。
