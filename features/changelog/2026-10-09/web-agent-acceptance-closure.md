Commit: 1728bd89cbc86119632b83f2cc0b67d7cf6fe810

# Agent 会话与模型弹窗修复验收闭环

完成 [Agent 会话结束后恢复回答](agent-chat-stream-completion.md) 与 [模型选择列表对齐](model-picker-layout.md) 的完整回归。事件流只有结束通知时补取保存的回答，读取失败或超过 15 秒时解除等待；模型单选项保持按钮与名称对齐，长名称在弹窗内换行。

补齐结果读取超时测试，并将 Team 提交测试限定到实际抽屉，核验重复点击只提交一次、重试保持同一执行 ID 与完整请求。浏览器在发送控件恢复后续聊，只统计回答气泡，覆盖 Agent 和 Operator 两种模式。相关旧测试按当前页面及接口约定修正，失败记录保留在仓库外。

## 测试覆盖

| 功能 | 测试名或验收 | 文件 |
| --- | --- | --- |
| 仅结束通知时恢复回答和输入 | `loads the saved answer and releases waiting when the only event is stream_end` | [completion.dom.test.jsx](../../../crates/web/spa/src/chat/stream/completion.dom.test.jsx) |
| 读取超时解除等待并显示错误 | `releases waiting with an error when the completion read exceeds fifteen seconds` | [completion.dom.test.jsx](../../../crates/web/spa/src/chat/stream/completion.dom.test.jsx) |
| 旧结果不能覆盖同一会话的新订阅 | `does not let a late completion replace a new subscription to the same conversation` | [completion.dom.test.jsx](../../../crates/web/spa/src/chat/stream/completion.dom.test.jsx) |
| 提交 Team 后打开执行明细 | `dispatches a team execution on confirm and opens its detail drawer` | [team.dom.test.jsx](../../../crates/web/spa/src/team.dom.test.jsx) |
| 重试不重复生成执行 ID | `shows an uncertain submission inside the drawer and retries the same ID with the draft intact` | [teams.dom.test.jsx](../../../crates/web/spa/src/fleet/teams/teams.dom.test.jsx) |
| 两种会话模式正常续聊、空事件流恢复第三次回答 | `chat` | [chat.js](../../../scripts/acceptance/ui/scenarios/chat.js) |
| 模型按钮对齐、长名称换行、名称点击与键盘选择 | 两种模式 × 四种屏宽，8 组浏览器验收 | [验收摘要](../../../../../opencoder-web-closure-20261009-132938/model-picker/summary.json) |

- `cargo test --workspace --no-fail-fast`：5729 passed / 0 failed / 8 ignored。忽略项为仓库已有的环境专项用例；其中 Brain 浏览器用例由全站验收单独执行并通过。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo build --workspace`、二进制及示例构建均通过。
- SPA 全量：156 个文件、1114 项通过；构建、产物一致性与 Ontology 类型检查通过。
- 平台及发布脚本回归：Python 155 项、Node 验收辅助测试 16 项通过。
- 全站 UI 的 16 项检查通过，覆盖 17 个注册页面、1920/1280/768/390 四种屏宽、真实 Server/节点/runc 流程及 Server TUI。
- 独立环境平滑切换与回滚验收通过；观察满 900 秒，54 个观察任务全部完成，最大受理延迟 3.141 秒、最大调度间隙 4.348 秒，均低于 30 秒门槛。测试进程和本轮创建的临时服务已清理。

完整结果、日志摘要与证据路径见 [验收回执](../../../../../opencoder-web-closure-20261009-132938/verification.json)。验收使用固定源码快照，修复及相关回归文件已核对与工作区一致；快照的完整文件摘要见回执关联的 manifest。本次未执行生产发布。

## 相关索引

- [web 模块](../../../agents/web/index.md)
- [Agent 调度平台](../../agent-platform/index.md)
