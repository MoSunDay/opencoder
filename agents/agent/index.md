Commit: (working-tree, 基于 c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6)

# agent — opencoder-agent 二进制

`crates/agent` 解析 remote、token、名称、工作目录、独立 data-dir、max-runs 和 DAG 开关，构造 [worker](../worker/index.md)，然后运行 [node](../node/index.md) 的出站 WebSocket 通道。

节点凭据来自参数或 `OPENCODER_SERVER_TOKEN`，不自动生成。正常关闭取消本地活动执行并等待有界收尾；异常退出后的未完成任务在重启时标记 interrupted，不自动重跑。

`internal-python-step` 是隐藏的隔离 VM 入口：在日志、Tokio 节点运行时、网络和配置初始化前短路，只通过标准输入/输出交换步骤数据。没有新增第三个平台二进制。

`dag prepare-rootfs --out DIR` 保留离线 rootfs 脚手架入口，不需要 Server 或模型。实际执行与恢复见 [worker](../worker/index.md)。
