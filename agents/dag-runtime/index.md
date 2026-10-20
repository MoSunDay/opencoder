Commit: 0047ac2515f865d7f33f99cc22764a1e80765391

# dag-runtime 模块

节点侧 DAG 调度与执行。一次运行归属一个节点、一个 `runc` 容器；Server 不链接此运行时。

## 主要流程

1. [resources.rs](../../crates/dag-runtime/src/resources.rs) 固定本次二进制、Agent 和所需依赖的版本与摘要。[layout.rs](../../crates/dag-runtime/src/layout.rs) 用创建时间的 UTC 日期和 DAG 标识生成稳定目录。
2. [sandbox/run](../../crates/dag-runtime/src/sandbox/run/mod.rs) 校验只读 NFS 源和镜像，在节点本地建立 OverlayFS 写层，再启动共享容器。源路径不移动、不改权限、不整目录复制；镜像与步骤元数据只读挂载。
3. [runtime.rs](../../crates/dag-runtime/src/runtime.rs) 与 [scheduler.rs](../../crates/dag-runtime/src/runtime/scheduler.rs) 按依赖调度。静态步骤与动态实例共享 `max_concurrency` 名额；动态展开清单先落盘，再按实例恢复。
4. [exec/native](../../crates/dag-runtime/src/exec/native/mod.rs) 与 [exec/agent_runc.rs](../../crates/dag-runtime/src/exec/agent_runc.rs) 都通过同一容器执行。工作目录为 `/workspace/<step>` 或 `/workspace/<step>/instances/<index>`，步骤之间没有额外安全边界。
5. [sandbox/supervisor.rs](../../crates/dag-runtime/src/sandbox/supervisor.rs) 管理步骤进程树。单步取消只终止该步骤；结束运行才关闭容器。恢复核验进程身份并清理遗留容器与挂载，不从新配置重新选择路径或版本。

[sandbox/run](../../crates/dag-runtime/src/sandbox/run/mod.rs) 等待内核卸载完成后才标记清理结束，随后才能提交终态并释放容量；卸载错误保留清理归属供重试，不按固定超时提前释放。

[runc/cleanup.rs](../../crates/dag-runtime/src/sandbox/runc/cleanup.rs) 回收删除进程并核验状态目录消失；若 runc 自身的退出期限先到，[init_exit.rs](../../crates/dag-runtime/src/sandbox/runc/init_exit.rs) 核对容器 ID、PID 与启动时间，仅等待正在退出或已收到 SIGKILL 的原初始化进程，再重试删除。普通运行中的进程、复用的 PID 和真实删除错误不会被当作清理成功。

## 输入、输出与资源

- [runtime/managed](../../crates/dag-runtime/src/runtime/managed/mod.rs) 保存单步暂停和等待恢复状态；[control](../../crates/dag-runtime/src/control/mod.rs) 保存命令、回执和历史。未决步骤不进入依赖终态表，暂停回执须等执行器返回；动态组只操作未完成实例。[exec/recovery](../../crates/dag-runtime/src/exec/recovery/mod.rs) 恢复原 Agent session，核对已有结果并继续原消息历史。接口见[单步恢复](../../docs/dag-step-control.md)。

- [StepCtx](../../crates/dag-runtime/src/exec/mod.rs) 从运行的冻结输入生成 context：静态二进制、Agent 与动态实例都包含 `run_id`、完整 `inputs` 和声明依赖的 `steps`，实例另有 `instance.index` 与 `instance.input`。[native/io.rs](../../crates/dag-runtime/src/exec/native/io.rs) 写入只读步骤元数据；Agent 提示词读取同一份 context。
- [runtime/input.rs](../../crates/dag-runtime/src/runtime/input.rs) 在启动容器前独占发布完整 `inputs.json`，与受理的 `input.json` 比较；恢复拒绝快照缺失、链接、非法 JSON、容量超限或输入漂移，不按变更后的文件重建快照。全部步骤与动态展开读取这一份冻结值。
- [resources.rs](../../crates/dag-runtime/src/resources.rs) 的 `frozen_resources` 只读取本次 `resources.json`，限制为 32 MiB 普通文件并拒绝软链接；缺失和损坏分别返回未准备与错误，不读取池的当前版本作为替代。
- [exec/how_copy.rs](../../crates/dag-runtime/src/exec/how_copy.rs)：在运行副本上追加 `how_append` 与实例文本，不回写资源池。
- [exec/native/artifacts.rs](../../crates/dag-runtime/src/exec/native/artifacts.rs)：共用 [声明校验](../../crates/dag/src/artifacts/declarations.rs)，只归档明确声明的任务产物文件，核对路径、大小和摘要；大文件流式复制。
- [step_log.rs](../../crates/dag-runtime/src/step_log.rs)、[dag_events.rs](../../crates/dag-runtime/src/dag_events.rs)：有界输出落库与实例事件。二进制输出写 `step_output`，Agent 事件通过 [runc_events.rs](../../crates/dag-runtime/src/exec/runc_events.rs) 导入子会话。
- [agent_runc.rs](../../crates/dag-runtime/src/exec/agent_runc.rs) 在启动 Agent 和事件读取器之前重置本次工作副本的事件流；[native/files.rs](../../crates/dag-runtime/src/exec/native/files.rs) 先拒绝软链接与非普通文件，再截断。事件持久化失败只停止内部执行进程，保留错误状态，不传播为用户取消。
- [exec/private_files.rs](../../crates/dag-runtime/src/exec/private_files.rs)：私有任务文件只读挂载到 `/run/opencoder-task`，模型只接收目录路径；系统不自动归档私有目录，公开记录不返回私有内容。
- Agent 资源与知识库分别使用容器内 `/run/opencoder/agents` 和 `/run/opencoder/knowledge`，不占用步骤名称。
- [sandbox/run/config.rs](../../crates/dag-runtime/src/sandbox/run/config.rs) 为 `/dev/pts` 建立独立 devpts 实例。知识根保持只读 bind，并按 [mounts.rs](../../crates/dag-runtime/src/sandbox/run/mounts.rs) 解析的当前 mountinfo 将根内子挂载逐项只读挂载；其他资源根仍为单层 bind。

## 执行器与镜像

- 原生 Agent 使用节点固定的模型配置；纯二进制和纯 Codex DAG 不额外要求原生模型凭证。
- [sandbox/codex](../../crates/dag-runtime/src/sandbox/codex/mod.rs) 校验容器内 Codex CLI 与启动脚本入口、固定 Harness/profile，并挂载实际执行节点的登录目录以支持认证刷新。脚本在同一容器执行；Server 不分发自身登录文件，配置或认证失败不会退回宿主执行。
- [startup.rs](../../crates/dag-runtime/src/sandbox/codex/startup.rs) 根据只读镜像中的 managed startup 摘要校验同版宿主代码，冻结该运行的启动配置。代理、部署声明、CLI 与 `connection_files` 显式声明的连接及其确切 token 文件通过最小只读私有挂载提供，不挂载整个客户端配置目录。共享 Codex home 保留原绝对路径；SQLite 必须位于该 home 内。
- `connection_files` 是可选绝对路径数组，省略或为空时不增加连接。连接与其 `token_file` 必须是已有普通文件，拒绝软链接与缺失文件；不按外部工具名称查找路径。真实只读挂载验证见 [connection-mount-probe](../../crates/dag-runtime/examples/connection-mount-probe.rs)。
- [broker.rs](../../crates/dag-runtime/src/sandbox/codex/broker.rs) 在宿主为每次 runc 执行启动私有 Unix socket broker。容器只请求账号 slot，宿主以宿主 PID 共用既有锁与账本。wrapper 退出不释放占用，步骤 supervisor 完成进程树回收后 Runtime 才发送结束状态；Runtime 异常断开时 broker 保留到对应容器消失。broker 不执行 Agent，不暴露宿主 `/proc`。
- 镜像的 `dag-runner` 和 `agent-step-runner` 必须与节点完整构建信息一致，不能是软链接。[制备脚本](../../scripts/prepare-dag-rootfs.sh) 安装运行器、Shell、Git、TLS、NSS 与 Python 依赖。
- [install-python.sh](../../scripts/dag-rootfs/install-python.sh) 要求 Python ≥ 3.11，保留解释器和随附动态库的目录布局，安装标准库、系统 CA 与时区数据；`prepare-dag-rootfs.sh --python` 可显式选择解释器。镜像校验在独立挂载命名空间提供只读设备，不把设备写入镜像。
- [install-codex.sh](../../scripts/dag-rootfs/install-codex.sh) 在写镜像前核验同目录的 Codex 与 `codex-code-mode-host` 原生 ELF，安装两者及动态库并验证伴随程序入口；[install-tools.sh](../../scripts/dag-rootfs/install-tools.sh) 提供 `rg`、`printenv` 等 Shell 工具。登录仍来自执行节点的原授权目录。
- [install-codex-startup.sh](../../scripts/dag-rootfs/install-codex-startup.sh) 按 profile 的绝对路径安装同版七个 wrapper 文件、Codext 及伴随程序；不复制配置、凭据或运行数据。带 managed startup 的发布镜像同时固定该发布的 `opencoder-cli`，历史无该声明的镜像保持原合同。
- [codex-startup-probe](../../crates/dag-runtime/examples/codex-startup-probe.rs) 使用隔离额度与进程 fixture，在真实 runc 验证私有 socket、并发分配、平台 session/thread/input、退出码及取消清理；真实模型验收另行执行。
- [tests/sandbox_environment.rs](../../crates/dag-runtime/tests/sandbox_environment.rs) 通过真实 runc 验证 PTY 数据传输、嵌套知识目录及文件只读、其他根的挂载范围和资源回收。
- [tests/preflight.rs](../../crates/dag-runtime/tests/preflight.rs)、[tests/run_loop](../../crates/dag-runtime/tests/run_loop/main.rs) 和 [两节点验收](../../scripts/acceptance/runc_scheduling/main.py) 覆盖版本拒绝、共享容器与实际恢复。

## 相关
- [sandbox/codex](../../crates/dag-runtime/src/sandbox/codex/mod.rs) 将固定的 Codex 启动脚本参数传入原生步骤；执行仍使用工作流容器和节点上的入口。


- [执行约定](../../rules/04-dag-execution-contract.md)、[DAG 能力](../../features/dag/index.md)
- [dag-binary](../dag-binary/index.md)、[worker](../worker/index.md)
- [动态步骤说明](../../docs/dag-dynamic.md)、[Codex 与 rootfs 配置](../../docs/registered-runners.md)
