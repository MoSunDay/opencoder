# fix(tui): `[turn cost]` 计时贯穿整个 turn，轮次间隙不再消失

## 背景

信息区尾部的 `[turn cost]` 计时器此前绑定到 per-round 锚点 `llm_round_started_at_ms`：一轮
（模型调用 + 其触发的全部 function call）在 `LlmRoundStart` 设锚、`LlmRoundEnd` 清锚。于是在
**轮次间隙**（上一轮的 tool 执行结束到下一轮模型调用开始之间）锚点为 `None`，`display_tail_ms`
返回 0，计时器从屏幕上消失、下一轮才重新出现。

用户期望的是 **whole-turn** 计时：从 prompt submit 到 `Done`，横跨所有 round 不中断。这与 footer
的 task-total 时长形成两个独立指标（turn 级 vs session 级）。

## 变更

- **`crates/tui/src/chat_types.rs`**：`ChatView` 新增 `turn_started_at_ms: Option<i64>` 字段
  （display-only，不参与 Serialize/持久化，derive `Default` 自动 `None`，向后兼容）。
- **`crates/tui/src/chat.rs`**：
  - `begin_turn`（prompt submit 入口）设 `turn_started_at_ms = Some(now_ms)`。
  - `SessionEvent::Done` / `SessionEvent::Error` 清除 `turn_started_at_ms = None`。
  - `LlmRoundEnd` **不**触碰该锚点——这是修复的核心：turn 锚点在轮次边界存活。
- **`crates/tui/src/app_display.rs`**：`display_tail_ms` 顶层运行路径改用新函数 `live_turn_ms`
  （基于 `turn_started_at_ms`），替代原 `live_round_ms`。subagent-focus 分支未改（仍用
  `live_round_ms`，保留 per-subagent 语义）。`live_round_ms` 保留供 subagent 路径使用。
- **`crates/tui/src/render.rs`**：尾部计时器注释更新为 whole-turn 语义。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 顶层运行态用 turn 锚点计 | `top_level_uses_turn_anchor` | app_display.rs |
| 轮次间隙（round 锚点 None）仍显示计时 | `between_rounds_keeps_turn_timer` | app_display.rs |
| 无 turn 锚点时返回 0 | `no_turn_started_has_no_timer` | app_display.rs |
| begin_turn 设锚 → LlmRoundEnd 保留 → Done 清除 | `turn_anchor_survives_round_end` | chat_tests/timer.rs |

> 原 `between_rounds_has_no_timer`（断言间隙 == 0，描述旧行为）被拆为
> `between_rounds_keeps_turn_timer`（断言 == 4000，验证修复）+ `no_turn_started_has_no_timer`
>（断言 == 0，覆盖无锚点边界）。`top_level_uses_only_the_live_llm_round` 重命名为
> `top_level_uses_turn_anchor` 并改用新字段。

- 全量回归：`cargo test --workspace` → **2231 passed / 0 failed / 0 ignored**
- tui lib：`cargo test -p opencoder-tui --lib` → **1161 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）
- build：`cargo build --workspace` → 零错误（EXIT=0）
- 行数：chat_types.rs 197；chat.rs 795（≤ 800）；app_display.rs 219；chat_tests/timer.rs 182；render.rs 799（≤ 800，本次仅注释 +0 净行）

## Impact Surface
- `display_tail_ms` 签名不变，渲染层 `tail_ms` 接口不变。
- `ChatView` 仅 derive `Default/Clone/Debug/PartialEq`，新字段默认 `None` 自动兼容历史数据。
- subagent-focus 计时路径未改，行为不变。
- 纯 TUI 显示层变更，不触及 session runner / store 数据形状 / prompt 契约 / 跨进程恢复。
