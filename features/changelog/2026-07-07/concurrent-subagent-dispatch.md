Commit: (working-tree, pre-initial-commit)

# 工具/子 agent 并发派发（对齐 opencode）

## 背景
原 runner 在一轮 LLM 返回多个工具调用时**串行**执行：逐个 `.await`。多个 `task`（子 agent）派发尤其低效——彼此独立却排队等待。opencode 主线对同一批工具调用并发执行。本次对齐。

## 变更
- **共享事件 sink**（`crates/session/src/runner.rs`）：新增类型别名 `Sink<'a> = Arc<Mutex<&'a mut (dyn FnMut(SessionEvent) + Send)>>` + 辅助函数 `emit(&sink, ev)`。把借用的 `FnMut` 闭包包进 `Mutex`，使多个在飞 tool/subagent future 能安全并发 emit（emit 是快速 push，串行化可接受）。生命周期绑定调用方闭包——**无 `'static` 要求**，借本地状态的测试闭包不改即可用。`emit` best-effort：mutex 中毒（仅闭包内 panic 可致）时丢弃事件而非传播。
- **工具循环改 `FuturesUnordered`**（`run_loop`，第 215 行起）：一批 tool_calls 全部 push 进 `FuturesUnordered` 并发 `.await`；结果按原始调用 index 重排后构造 Tool message，保证 Tool message 内容与事件回放顺序**确定**（不受完成先后影响）。`ToolStart` 仍按调用顺序预声明；`ToolEnd` 按完成顺序 emit。**cancel 下不 break、drain 至完成**——否则丢弃在飞 subagent future 会漏发 `SubagentEnd`/`complete_subagent_task` 并留下无 result 的 tool_use；被取消的工具经 select!/child.cancel 快速收敛，run 在下一轮顶部 cancel 检查收尾。
- **签名下沉 sink**：`execute_call` / `run_subagent` 改收 `&Sink`；子 agent 事件转发器 `Arc::clone(sink)` 捕获，父子并发时父 sink 共享。`run_one_llm_call` 形参放宽为 `&mut (impl FnMut + Send + ?Sized)` 以兼容 `dyn FnMut`。
- **公共 API 不变**：`run` / `run_with_registry` 仍收 `impl FnMut + Send`；sink 包装为内部细节。
- **TUI 运行指示**（`crates/tui/src/chat.rs` + `render.rs`）：`ChatView.subagents_running` 计数器——`SubagentStart` saturating_add(1)、`SubagentEnd` saturating_sub(1)、`Done`/`Error` 归零；状态栏显示 `↳sub:N running` 徽标（N>0 时）。CLI/TUI 原有 `SubagentStart`/`SubagentEnd` 文本标记保留。

## 涉及文件
- `crates/session/src/runner.rs` — `Sink`/`emit`（第 56/60 行）、`QueueConsumed`（第 38 行）、`run_loop` 工具 `FuturesUnordered`（第 215 行）、`execute_call`/`run_subagent`/`run_one_llm_call` 签名
- `crates/tui/src/chat.rs` — `subagents_running` 字段 + inc/dec/reset
- `crates/tui/src/render.rs` — 状态栏 `subagents_running` 徽标
- `crates/session/tests/subagent.rs` — `concurrent_subagent_dispatch_in_one_turn` 结构性测试

## 兼容性 / 注意
- **回放非确定性**：并发下 `session_events` 按完成顺序持久化，跨 run 的逐字节回放顺序不再确定（结构上并发，功能正确）。MVP 接受此限制。
- **并发是结构性的**：测试证明「一轮多 task 各自跑完」成立；MockChatClient 经 `Mutex` 守护的 FIFO 串行化响应，故不验证真实时序并发（时序并发取决于真实 LLM/工具 IO）。
- `emit` 中毒丢事件：仅闭包内 panic 可触发，正常路径不达。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 一轮多 task 并发派发 | `concurrent_subagent_dispatch_in_one_turn` | `session/tests/subagent.rs` |

## gate
- `cargo test --workspace` → 258 passed / 0 failed
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告

## 相关文档
- [agents/session](../../../agents/session/index.md) — 并发工具派发主流程
