Commit: (working-tree, pre-initial-commit)

# refactor(tui): `[turn cost]` 改为 per-LLM-interaction 计费（freeze-then-reset）

## 背景

上一版（`tui-turn-cost-persists-across-rounds.md`）引入了 `turn_started_at_ms` 整轮跨度锚点，
让计时器在轮间不消失。但实际使用暴露**三个独立 bug**：

1. **`render.rs` `end == n` 门控**：内容滚动离开尾部 / 内容抖动导致 `end != n` 时计时器消失，
   而 spinner 仍在转——视觉不一致。
2. **viewport 裁剪**：内容填满 viewport 后 timer 追加到第 `visible_h+1` 行被 ratatui 裁剪——
   "又消失了"的另一元凶。
3. **语义不一致**：顶层用 `turn_started_at_ms`（整轮跨度），subagent 聚焦用 `llm_round_started_at_ms`
   （单轮），两者行为不一致且不符合 per-LLM-interaction reset 语义。

本次改为 **freeze-then-reset** 模型：一轮 LLM 结束 → 冻结该轮耗时显示 → 下一轮 `LlmRoundStart`
时 reset 归零重新计时。顶层与 subagent 聚焦统一。

## 变更

### 字段重构（`chat_types.rs`）
- **删除** `turn_started_at_ms: Option<i64>`
- **新增** `frozen_round_ms: Option<u64>`：`LlmRoundEnd` 冻结的该轮最终耗时

### 事件处理（`chat.rs`）
| 事件 | 新行为 |
|------|--------|
| `begin_turn` | 设 `llm_round_started_at_ms = now`（submit 即计时），清 `frozen_round_ms` |
| `LlmRoundStart` | 设 round anchor + 清 `frozen_round_ms`（reset） |
| `LlmRoundEnd` | **冻结** `frozen_round_ms = now - anchor`，再清 anchor → timer 冻结不消失 |
| `Done` / `Error` | 清 round anchor + 清 `frozen_round_ms` |
| `recover_round_anchor_if_missing` | 补设 anchor + 清 `frozen_round_ms` |
| `SubagentStart` | 子 view seed `llm_round_started_at_ms = now` |
| `mark_subagent_done` | 清 round anchor + 清 `frozen_round_ms` |

### 统一计时逻辑（`app_display.rs`）
- 删除 `live_turn_ms` + `live_round_ms`
- 新增统一函数 `round_or_frozen(chat, now)`：
  - round anchor `Some` → live: `now - anchor`
  - round anchor `None` → `frozen_round_ms.unwrap_or(0)`
- `display_tail_ms` 顶层 + subagent 分支统一调用 `round_or_frozen`

### 渲染防裁剪（`render.rs`）
- **删除** `&& end == n` 条件 → timer 不再因滚动位置而消失
- **预留行防裁剪**：`tail_ms > 0` 时 `content_h = visible_h - 1`，timer 渲染为独立 bottom row
  （与 content Paragraph 分离渲染），永不被内容挤出 viewport
- scrollbar 判定仍用原始 `visible_h`（避免闪烁）；thumb 计算用 `content_h`

### 其他
- `chat_helpers.rs::reconcile_orphaned_subagents`：同步清 `frozen_round_ms`

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 活跃轮 live 计时 | `top_level_live_round_counts_up` | app_display.rs |
| 轮间冻结显示上一轮耗时 | `between_rounds_freezes_last_round_cost` | app_display.rs |
| 无锚点无冻结 → 0 | `no_round_and_no_frozen_has_no_timer` | app_display.rs |
| freeze→reset 全生命周期 | `frozen_round_persists_after_round_end` | chat_tests/timer.rs |
| LlmRoundEnd 冻结 | `llm_round_lifecycle` 新增 frozen 断言 | chat_tests/mod.rs |
| 滚动状态 timer 仍渲染 | `body_timer_visible_when_scrolled_away_from_tail` | render_tests/timer.rs |
| timer 独立行不混入内容 | `body_turn_cost_timer_on_own_line` 等既有 | render_tests/timer.rs |

> 替换的旧测试：`turn_anchor_survives_round_end` → `frozen_round_persists_after_round_end`；
> `top_level_uses_turn_anchor` → `top_level_live_round_counts_up`；
> `between_rounds_keeps_turn_timer` → `between_rounds_freezes_last_round_cost`；
> `no_turn_started_has_no_timer` → `no_round_and_no_frozen_has_no_timer`。

- 全量回归：`cargo test --workspace` → **2227 passed / 0 failed / 0 ignored**
- tui lib：`cargo test -p opencoder-tui --lib` → **1157 passed / 0 failed**
- clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告（EXIT=0）
- build：`cargo build --workspace` → 零错误（EXIT=0）
- 行数：chat_types.rs 201；chat.rs 779（push_duration_span 移至 chat_helpers，迭代文件 ≤800 ✅）；
  chat_helpers.rs 175（接收 push_duration_span）；app_display.rs 221；render.rs 743；
  chat_tests/timer.rs 191；render_tests/timer.rs 220

## Impact Surface
- `display_tail_ms` 签名不变，`tail_ms` 渲染接口不变。
- `ChatView` derive `Default`，新字段 `frozen_round_ms` 默认 `None` 自动兼容历史数据。
- 顶层与 subagent 聚焦计时语义统一（freeze-then-reset）。
- 纯 TUI 显示层变更，不触及 session runner / store 数据形状 / prompt 契约 / 跨进程恢复。

## 与上一版的关系
取代 `tui-turn-cost-persists-across-rounds.md` 的 whole-turn 模型。该模型的三个 bug
（门控消失 / 裁剪 / 语义不一致）全部修复。
