Commit: 0bc5b867a766100422dd4d0cd214a57f794db26d (working-tree)

# Agent 配置详情抽屉与 Wrap 参数收敛

Agent 列表只展示名称、生效标记和编辑／启动／删除操作。编辑从右侧滑出占视口 75% 的详情抽屉，关闭后继续使用原列表。资源引用、Prompt 编辑、历史以及默认 Harness／配置档案绑定均在详情中维护。

Harness 管理只编辑 `opencoder --wrap codex` 的 `--model` 和 `--envs` 输入。表单读取与提交均仅投影这两项，删除 Codex 二进制路径、授权槽位、推理强度、沙箱及审批策略控件；保存完整配置时由 API 将省略的 Codex 自身选项置为默认值。命名配置档案继续供 Agent 引用，运行中的配置快照不变。

删除重复的 Agent Harness 列表及 Runner 管理组件，宿主机执行继续使用 Operator 页签。此次修改限于 Web 管理交互；业务执行回执及服务端运行协议未变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|---|---|---|
| 精简列表、抽屉宽度、关闭后切换 Agent | `opens agent editing in a right-side 75 percent drawer` | `crates/web/spa/src/agentsConfig.dom.test.jsx` |
| 资源引用、Prompt 保存及回滚 | `AgentDetail` | `crates/web/spa/src/agentDetail.dom.test.jsx` |
| 内置／自定义 Agent 执行方式、档案绑定与清除、读取及保存错误 | `changes the execution method`、`binds and clears a named profile` 等 | `crates/web/spa/src/harness/agentFields.dom.test.jsx` |
| 仅提交 Wrap 参数、非法环境阻止保存、保存失败保留输入 | `edits managed Codex parameters`、`keeps model and environment edits after a failed save` | `crates/web/spa/src/harness/management.dom.test.jsx` |
| 命名配置保存及执行报告下载 | `creates a named Codex profile containing only wrap parameters` | `crates/web/spa/src/harness/runner.dom.test.jsx` |
| Operator 权限及启动流程 | `OperatorPanel launch flow` | `crates/web/spa/src/operators/panel.dom.test.jsx` |
| 浏览器抽屉、真实 API 保存、受控 Codex 进程、续聊与刷新 | `settings.js`／`codex.js` | `scripts/acceptance/harness/` |

最终工作区 Web 全量回归：83 个测试文件、674 项通过，包含并行进入工作区的 TODO 运行画布改动；SPA 构建通过，重建前后产物校验和一致。Clippy 零警告。日志分别为 `/tmp/opencoder-agent-web-final-tests.log`、`/tmp/opencoder-agent-web-build.log`、`/tmp/opencoder-agent-web-clippy.log`。

当前构建的 Server／Node 与内嵌 SPA 已通过 `scripts/acceptance/harness/codex.js` 浏览器验收：抽屉右对齐且宽度为视口 75%，真实 API 保存仅包含 Wrap 参数，受控 Codex 子进程读取环境变量并完成响应、刷新回放及续聊。该验收使用确定性 Codex fixture，不调用外部模型。日志：`/tmp/opencoder-agent-web-browser.log`；最终截图：`/tmp/opencoder-wrap-browser-cQI5qQ/`。

首次节点冒烟失败时，`tests/support/mod.rs::sibling_bin` 取到的是前一天的 Server／Agent 二进制。先重建 workspace 再运行同一冒烟测试已通过，日志为 `/tmp/opencoder-agent-web-nodes-fresh.log`；复验时须先构建这两个实际进程，不能把测试 harness 编译完成当成普通二进制已更新。

Rust 全量回归：`cargo test --workspace -j 12` → 5,051 passed / 0 failed / 6 ignored；6 项忽略均为仓库既有的特权 runc／NFS 环境测试，没有新增跳过项。日志：`/tmp/opencoder-agent-web-rust-tests.log`。回归后的 `cargo build --workspace -j 12` 通过，日志：`/tmp/opencoder-agent-web-rust-build-final.log`。

本轮未发布生产，也未操作现有鉴权数据。

相关本地记录：[Web 模块](../../../agents/web/index.md)、[Agent Harness](../../harness/index.md)。
