Commit: 4ae5b50508e9d9016edeb45c61361240ecce1e37

# TUI `[turn cost]` 改为 running 模型轮次计时

## Context

原实现把整个任务的 `running` 时段累计为一个 whole-turn 值，并在任务终态后冻结到 transcript 尾部。这会把同一任务中的多条 assistant 消息与多轮 function call 混成一个耗时，和逐轮 `turn cost` 语义不符。

## Change Summary

- 每次 provider/model 调用前开始一轮计时；该 assistant 消息请求的全部 function call 完成后结束并立即 reset。
- TUI 只在当前任务或聚焦 subagent 处于 running 且轮次活动时，把 `[turn cost x]` 放在最新可见消息末尾；轮间和任务终态不展示历史冻结值。
- 任务总耗时仍由独立 task clock 维护，不与逐轮耗时混用。
- 轮次时间只存在于生命周期事件和 TUI 展示状态，不写入 `Message`、提示词或模型 context。

## Compatibility

- 新增 `llm_round_start` / `llm_round_end` 细粒度 session 事件；既有 message schema、数据库表与模型请求格式不变。
- CLI 忽略这两个纯展示生命周期事件；subagent 通过既有 `SubagentChild` 路由获得独立轮次状态。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 一条 assistant 消息及其全部工具共用一轮 | `each_model_message_gets_one_round_covering_all_its_tools` | `session/tests/llm_round_lifecycle.rs` |
| 终态前结束计时 | `terminal_text_round_ends_before_done` | `session/tests/llm_round_lifecycle.rs` |
| TUI 仅展示活动轮次 | `top_level_uses_only_the_live_llm_round` / `between_rounds_has_no_timer` | `tui/src/app_display.rs` |
| SSE 新事件往返 | `from_sse_roundtrips_all_variants` | `session/src/runner/event.rs` |

## Gate

- 全量回归：`cargo test --workspace` → **2093 passed / 0 failed**（EXIT=0）。
- 静态检查：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）。
- 构建：`cargo build --workspace` → 成功（EXIT=0）。
- UI 定向：`app_display::tests` 7 项全部通过，覆盖运行、轮间、终态及 title 主题样式。

## Related Docs

- [tui logic](../../../agents/tui/index.md)
- [session logic](../../../agents/session/index.md)
