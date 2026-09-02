# TUI 轮计时器文案收敛——`[turn cost xs]` → `[call cost xs]`

Commit: bb986ff (标签语义对齐)

## 背景

- TUI 底栏计时器统计的是单次 LLM round（`LlmRoundStart`→`LlmRoundEnd`），即一次
  call，而非整个用户 turn（turn 内可含多个 tool 循环 round）。`[turn cost xs]`
  标签与实际语义不符，收敛为 `[call cost xs]`。
- 唯一渲染出口在 `crates/tui/src/theme.rs` 的 `rounded_block_line_tok`；其余
  全是注释与测试断言。

## 变更

- 文案本体：`theme.rs` `format!("[turn cost {}]"→"[call cost {}]")`（渲染出口唯一）。
- 测试断言同步（否则字面量断言全挂）：`render_tests/timer.rs`、
  `render_tests/tok_cost.rs` 的全部 `[turn cost…]` 断言与消息文案；
  `chat_tests/mod.rs` 的 `contains("turn cost")`；
  `crates/session/tests/llm_round_lifecycle.rs` 的 `contains("[turn cost")`
  与模块注释。
- 可检索性：测试函数名同步改名（`body_shows_call_cost_timer_at_content_tail`、
  `body_hides_call_cost_timer_when_zero`、`body_call_cost_timer_on_own_line`、
  `body_call_cost_timer_always_own_line`、
  `body_call_cost_timer_not_mixed_into_tool_output`、
  `tok_cost_border_appends_call_cost_segment_when_timing`）。
- 注释/文档一致性：`theme.rs`、`chat_types.rs`、`chat_helpers.rs`、
  `copy_mode/mod.rs`、`render.rs`、`chat_tests/timer.rs` 中对
  `[turn cost]` 的描述全部改 `[call cost]`。
- 不改：内部标识符 `turn_ms`/`show_turn`（内部命名重构不在范围）；历史
  changelog 不回写；`sidecar_loop.rs` 的 "per-turn cost" 是 turn 汇总语义，
  与本标签无关，不动。

## 回归

- `cargo test -p opencoder-tui`（改动落盘后全量）→ lib 编译通过、
  render_tests/chat_tests 单测与 integration（含 `tok_cost_replay` 7 passed、
  control_command 2 passed）全绿，Doc-tests 正常执行。
- `cargo test -p opencoder-session` → 全套件 0 failed（含
  `llm_round_lifecycle`）。
- `cargo test -p opencoder-session --test llm_round_lifecycle` 单独复跑 →
  4 passed / 0 failed。
- 并行重构说明：收尾时工作树中另一会话的 sidecar-destroy 重构（他人
  working-tree 改动）一度使 `opencoder-tui` lib 处于编译中间态（23:15
  首次完整绿测取证于其破坏编译之前）；该重构落定后复跑
  `cargo test -p opencoder-tui --lib` → **1595 passed / 0 failed**（含全部
  6 个改名 `call_cost` 测试），随后 `cargo test -p opencoder-tui` 全量
  0 failed。中途出现的 1 个瞬时失败在紧邻复跑中消失（多会话构建负载
  flake，非回归）。

## 判定

- 纯文案语义对齐，无生产逻辑改动；rules/02 测试清单如上。
