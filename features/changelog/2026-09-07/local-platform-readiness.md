Commit: 36e787432f7844ba5066433d36a43c24c05c115e

# 本机平台部署与 Web 单 Agent 验收

- Agent 配置 DOM 测试按可访问名称直接查询按钮，避免先遍历整页按钮并计算隐藏背景的可见性。保留原有 API 请求断言与超时；创建用例从约 28 秒降至约 1 秒。
- 移动导航的 CSS 选择器增加内容容器限定，避免 antd 动态样式覆盖桌面隐藏规则；窄屏仍显示分类与页面选择器。
- 节点冒烟脚本遵循标准 `TMPDIR`，允许在具备足够空间的文件系统运行，同时保留未配置时的 `/tmp` 默认值及完整节点健康检查。

## 测试覆盖

| 行为 | 测试 / 验收 | 入口 |
| --- | --- | --- |
| 创建 Agent 的引用参数 | `creates an agent through POST with null refs for untouched selects` | `crates/web/spa/src/agentsConfig.dom.test.jsx` |
| 启动 Agent 后展示执行详情 | `starts a configured agent and opens its node-owned execution` | 同上 |
| 节点注册、派发、归属明细和事件 | `smoke_script_two_process_nodes_flow_passes` | `tests/nodes_smoke_proc.rs` |
| 桌面隐藏移动导航、窄屏显示移动导航 | Chromium，1480 × 1050 / 390 × 844，检查计算样式及截图 | 本机部署验收记录 |
| Web 单 agent 工具调用、续写与历史恢复 | 真实 Server / Node / 模型，浏览器操作会话交互与全部执行 | 本机部署验收记录 |
| 节点离线与服务重启后的续写 | 离线明细及定向派发返回 503；重启保留节点 ID 和会话，显式复开接单后执行真实工具 | 本机部署验收记录 |

回归命令：

- `npm test`（SPA，全量 402 项）。
- `scripts/check-spa-drift.sh`。
- `cargo build --workspace --locked`。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`。
- `TMPDIR=/var/tmp cargo test --workspace --locked`。

本机的 `/tmp` 位于 `/data00`，可用块比例低于节点存储健康阈值。测试使用根文件系统上的 `/var/tmp`；正式 Node 持久化使用 `/var/lib/opencoder-node`，执行工作目录保持 `/data00/github/opencoder`。未降低健康阈值，未清理既有数据库。

全量回归通过：Rust 4,752 项通过、5 项既有手工测试忽略；SPA 42 个文件、402 项通过；Clippy 零警告，SPA 产物一致性检查通过。

## 本机运行与兼容

Server 在 18081 提供 Web，本机以 `human-os-02` 注册，Server 与 Node 均由 systemd 常驻并开机启动。Web 会话交互和全部执行两个入口均已通过真实模型与工具调用验收，包含同会话追问、刷新恢复历史和服务重启后续写。旧 8080 daemon 已停止，旧数据库与二进制保留。

正常停止会持久冻结接单；重启 Server 和 Node 后需显式复开。`active_executions` 包含等待继续的 idle 会话；停机前应检查节点 `active_runs` 与 `active_agent_loops`。本机运维说明、浏览器截图及验收 JSON 位于 `/data00/opencoder-delivery/20260907-local-18081/`，凭据仅保存在仓库外受限文件中。

## 相关文档

- [Agent 调度平台](../../agent-platform/index.md)
- [Web 模块](../../../agents/web/index.md)
- [Node 通信模块](../../../agents/node/index.md)
- [平台部署说明](../../../docs/agent-platform.md)
