Commit: 6bf573e09693caf9a00e37429f9a8f2d796dc219

# Agent 会话结束后恢复回答

事件流只有 `stream_end`、没有会话级 `done/error` 时，会话页曾持续显示“等待首个事件…”。传输层现在通知会话页读取执行结果与已保存的消息，恢复回答或显示启动失败原因，并解除等待。读取有超时限制，取消和重新订阅会使旧读取失效。

浏览器回归等待发送控件恢复后再续聊，覆盖真实节点保存回答、浏览器只收到结束通知的情况。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|---|---|---|
| 仅有结束通知时恢复回答和输入 | `loads the saved answer and releases waiting when the only event is stream_end` | [completion.dom.test.jsx](../../../crates/web/spa/src/chat/stream/completion.dom.test.jsx) |
| 未创建会话时显示启动错误 | `shows an initialization error even when no session transcript was created` | [completion.dom.test.jsx](../../../crates/web/spa/src/chat/stream/completion.dom.test.jsx) |
| 读取失败解除等待 | `shows a result-read failure instead of leaving the composer busy` | [completion.dom.test.jsx](../../../crates/web/spa/src/chat/stream/completion.dom.test.jsx) |
| 旧读取不能覆盖同一会话的新订阅 | `does not let a late completion replace a new subscription to the same conversation` | [completion.dom.test.jsx](../../../crates/web/spa/src/chat/stream/completion.dom.test.jsx) |
| 空事件流只通知结束，不伪造会话事件 | `notifies the owner of an empty completed stream without inventing a session event` | [sse.resume.test.js](../../../crates/web/spa/src/sse.resume.test.js) |
| 主动取消不当作正常完成 | `does not report completion when the caller aborts a stream` | [sse.resume.test.js](../../../crates/web/spa/src/sse.resume.test.js) |
| 浏览器续聊复用执行并补取第三次回答 | `chat` | [chat.js](../../../scripts/acceptance/ui/scenarios/chat.js) |

- SPA 全量：152 个文件，1073 项通过。
- Rust 全量 `cargo test --workspace --no-fail-fast`：5715 passed / 1 failed / 8 ignored；唯一失败是 `registered_runner_is_rejected_by_dag_definition_and_inline_dispatch` 等待测试节点就绪超时。同一测试程序单独重跑输出 `1 passed; 0 failed`，耗时 3.26 秒；保留原始失败记录，未将首次全量记为全绿。
- `cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、`cargo build --workspace` 均通过；发布脚本回归 58 项通过。
- 全站 UI 入口的 16 项检查通过，包含 SPA 一致性、1920/1280/768/390 像素巡检、真实功能流程及 TUI。独立浏览器同时验证了 Agent、Operator 的首次提交、正常回复和续聊。

## 相关索引

- [web 模块](../../../agents/web/index.md)
- [Agent 调度平台](../../agent-platform/index.md)
