Commit: pending

# 记忆维护过程全程回显

## 背景

`local-memory` 维护运行时父会话只收到一行 `Status("updating local memory")`；子会话的工具调用、输出与文本在转发闭包中被丢弃，用户看不到维护实际做了什么。

## 变化

- `runner/local_memory::after_task` 复用 subagent 展示路由回显维护过程：维护开始发 `SubagentStart`（kind `memory`，id 即 `memory-<id>` 子会话 id），子会话事件统一包裹为 `SubagentChild` 转发（含 `ToolStart`/`ToolEnd`/`TextDelta`/`LlmUsage` 等），结束时发 `SubagentEnd` 携带摘要（文本流前 240 字符）或失败原因。
- `SubagentEnd` 在成功、子错误、运行失败所有退出路径闭合：不闭合会卡住父视图的 running 计数器直到下一个 `Done`。
- `LlmUsage` 由裸转发改为包裹转发，token 仍折算进父会话成本（TUI 对 `SubagentChild(LlmUsage)` 本就折叠），行为等价且子块内也可见。
- 前端零新增接线：TUI 折叠块、Web SSE 中继、headless 尾注复用既有 `SubagentStart/Child/End` 渲染。

## 影响

仅展示层事件路由；维护触发判定、独立消息历史、结束时机契约不变。详情见 [本地仓库记忆](../../local-memory/index.md)。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 维护开块/包裹流/闭块完整契约 | `memory_run_echoes_its_progress_as_a_subagent_block` | `crates/session/src/runner/local_memory/mod.rs` |

- 定向回归：`cargo test -p opencoder-session --lib` 475 通过；`cargo check --workspace --all-targets` 通过。
