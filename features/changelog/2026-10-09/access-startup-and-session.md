Commit: df06a3b0a177c14b53171818717eaca0e0c35e64

# 用户权限、启动入口与会话输入

用户角色统一为 admin、editor、viewer，用户与 Token 分开管理。schema v34 保留已有凭据摘要，发布数据格式提升为 5。Codex 支持固定的启动命令数组；Agent 容器会话支持引导、排队和原执行续会话。项目详情与对话入口继续复用原生执行，结论从所属节点实时读取。

| 功能 | 验证入口 |
| --- | --- |
| 独立 Token、有效期与即时角色校验 | [access_tokens.rs](../../../crates/store/tests/access_tokens.rs)、[users_api.rs](../../../crates/control/tests/e2e/users_api.rs) |
| Codex 参数、续会话、分叉、失败与取消 | [harness_startup.rs](../../../crates/session/tests/harness_startup.rs) |
| 容器引导、排队和重复输入 | [sandbox_inputs.rs](../../../tests/operator_e2e/sandbox_inputs.rs) |
| 项目对话沿用原执行 | [conversation.rs](../../../crates/ctl/tests/project/conversation.rs)、[能力关联验收](../../../scripts/acceptance/project/capabilities/main.js) |
| 权限页面、四种屏宽与终端交互 | [全站验收](../../../scripts/acceptance/ui/main.js) |

验证通过：Rust workspace 5,725 项、SPA 1,092 项、Python 227 项；格式检查、Clippy、SPA 类型与产物检查通过。原生 NFS、runc、旧 Runtime、全站浏览器与 TUI，以及升级回滚和 15 分钟观察均通过。

实现索引：[core](../../../agents/core/index.md)、[store](../../../agents/store/index.md)、[session](../../../agents/session/index.md)、[control](../../../agents/control/index.md)、[worker](../../../agents/worker/index.md)、[Web](../../../agents/web/index.md)。
