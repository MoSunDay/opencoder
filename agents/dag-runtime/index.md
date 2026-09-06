Commit: (working-tree, 基于 c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6)

# dag-runtime — 节点侧 DAG 执行运行时

[worker](../worker/index.md) 通过 `Uplink::for_local_dag` 将事件和状态写入 Node Store。Server 只接收执行索引；平台的 VM/runc 执行由 `opencoder-agent` 承担，`opencoder-server` 不链接该运行时。

## 执行与恢复

- `runtime.rs` 验证定义、并发调度至多 4 个步骤，传播取消令牌，持久化步骤输出并折叠终态。Python 自己处理预算与取消，外层不能提前丢弃清理 future。
- `checkpoint` 恢复 `meta.json` 为 done 且产物可读的步骤；先 fsync 输出，最后写 meta。恢复只由显式 resume 触发，已完成步骤不重跑。
- `step_io` 统一记录产物和事件；写入失败使步骤失败并阻断依赖。节点本地事件持久化错误由适配器锁存并返回。
- 旧 REST Uplink 保留兼容与测试用途；终态上报短退避重试一次，仍失败则向执行 owner 返回错误，不再只 warn 后报告正常返回。`dag_events` 批量队列按 8 条或 300ms 刷新，终态前等待关闭。

## Python 生命周期

`exec/python/mod.rs` 执行内嵌 RustPython，StringIO 捕获输出，注入 `RUN_ID`、`STEP_DIR`、`context`，解析可选 `output.json`。保留配置值 `sandbox: in_process`，实际进程隔离由 `exec/python/process.rs` 实现：同一个 Agent 二进制的隐藏入口通过 stdin/stdout 交换请求与结果。发送完请求关闭 stdin，取消/超时先停止进程组并回收直接子进程，再返回终态。Linux 设置父进程死亡信号。

库测试和其他嵌入者需要同构建目录或相邻目录的 `opencoder-agent`，缺失时明确报错；先构建 workspace。VM 不安装宿主信号处理器；不含完整 Python stdlib（有 `_io/itertools/posix`，无常用 `json/math`）。

`sandbox/runc` 使用独立 bundle 状态目录，取消/超时执行有界 force delete 并回收 launcher，清理错误返回调用方。`sandbox/rootfs` 为每个 bundle 复制独立解释器树并复用到重试，重建空运行时目录，避免并发初始化共享 rootfs 的设备文件。复制在 blocking 工作线程执行；运行时以只读 root 挂载，预留每步一份解释器树磁盘空间。容器 ID 是 run 与 step 的组合，允许两个合法的长 ID 组合。`sandbox/oci` 生成只读 rootfs 和可写 run bind；rootfs 必须为真实目录，不能是 symlink。需要完整 Python 时在 rootfs 中部署解释器。

## 验证入口

- VM 正常输出、异常、超时和无限循环取消：`exec/python/tests.rs`。
- 状态上报故障、依赖阻断和取消 drain：`tests/run_loop.rs`。
- 真实 runc：显式执行 `sandbox::runc::tests` 的 manual 测试，并设置已有测试变量 `DAG_TEST_ROOTFS`；缺失前提会失败，不静默跳过。
