# 本机平台部署前的 Web 与节点冒烟修正

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

回归命令：

- `npm test`（SPA，全量 402 项）。
- `scripts/check-spa-drift.sh`。
- `cargo build --workspace --locked`。
- `cargo clippy --workspace --all-targets --locked -- -D warnings`。
- `TMPDIR=/var/tmp cargo test --workspace --locked`。

本机的 `/tmp` 位于 `/data00`，可用块比例低于节点存储健康阈值。测试使用根文件系统上的 `/var/tmp`；正式 Node 持久化使用 `/var/lib/opencoder-node`，执行工作目录保持 `/data00/github/opencoder`。未降低健康阈值，未清理既有数据库。
