Commit: c799d510

# 记忆维护：计时、成本可证与上下文隔离

## 背景

用户在真实会话的维护回显块里看到工程约束式叙述，提出三点要求：维护执行计时、token 计入任务成本可证、维护产物绝不进入主任务上下文；稳定记忆文档需清除工程约束描述，防止污染后续提示词。

## 变化

- 维护执行时长以 `(<dur>)` 前缀进入 `SubagentEnd` 摘要（复用 `execute.rs` 的 `fmt_dur`，成功路径；失败摘要保持原始错误文本）。
- 断言维护轮 `LlmUsage` 以 `SubagentChild` 包裹转发——即 TUI 既有折叠路径的输入，token 计入父视图成本；新增 TUI 钉子测试。
- 新增内容级隔离测试（维护输出与维护指令均不出现在主会话任何消息）+ 真实 DB 验证说明：`memory-*` 会话/消息行数为 0，回显仅存在于 session_events 展示流。
- `features/local-memory` 与 `agents/session` 稳定文档去除路由/事件名等工程约束描述，保留能力事实。
- TUI 会话切换回归测试补 HOME 隔离——此前 `Config::load` 合并真实 `~/.opencoder` 全局配置，local-memory 开启的机器上维护子会话在测试内触发第二次 LLM 调用（`switched_model_used_by_next_turn` 误报）。

## 影响

展示层与文档层面，无行为契约变化。详情见 [本地仓库记忆](../../local-memory/index.md)。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 维护 LlmUsage 以 SubagentChild 包裹转发 + 摘要时长前缀 | `memory_run_echoes_its_progress_as_a_subagent_block` | `crates/session/src/runner/local_memory/mod.rs` |
| 维护输出与指令绝不进入主会话消息 | `memory_output_never_enters_the_parent_transcript` | `crates/session/src/runner/local_memory/mod.rs` |
| act 任务后正常维护（既有，回归护栏） | `enabled_memory_uses_a_context_copy_after_main_completion` | `crates/session/src/runner/local_memory/mod.rs` |
| TUI：memory 块 token 折入父视图成本 + 块收口时长 | `memory_block_cost_and_duration_land_in_parent_view` | `crates/tui/tests/tok_cost_replay.rs` |
| 会话切换测试的环境隔离 | `switched_model_used_by_next_turn`（加固） | `crates/tui/tests/session_switch_restore.rs` |
