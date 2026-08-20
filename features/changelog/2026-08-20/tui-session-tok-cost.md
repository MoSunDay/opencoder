# TUI 信息区第四角：底边框左下 `[tok cost X.XXXm]` 会话累计真实 token 消耗

## 背景

信息区四角此前只有三角：左上 body 标题、右上 ⬆、右下跟随指示；`[turn cost]`
位于内容末行（非边框角）。真实 usage 只存在于持久化的 `assistant Message.usage`，
TUI 完全没消费，`LlmRoundEnd` 事件无载荷。本次补齐第四角：body 圆角块**底边框
左下角**渲染 `[tok cost X.XXXm]`——当前 session 累计消耗的真实 token 总量
（m=百万，三位小数，正值守 0.001m 下限，空 session 显示 `0`）；当轮次计时非零时
同一角标以 `·` 追加 `[turn cost …]` 段。两段均为默认前景色，copy 模式自动隐藏。

## 实现

- **事件层**（`crates/session/src/runner/event.rs`）：`SessionEvent` 新增
  `LlmUsage { total_tokens: u64 }`；kind `"llm_usage"`、payload、`from_sse` 解码、
  `coarse_kind → EventKind::Step`（DB `type` 列向后兼容）。roundtrip 测试的
  kind 去重计数 21 → 22。
- **发射点**（`crates/session/src/runner/mod.rs::run_loop`）：assistant 消息
  （含 `usage`）持久化后立即发射，早于 `LlmRoundEnd`；provider 未返回 usage 的
  轮次不发射。compaction 摘要调用不产生 assistant 消息、不发射——live 与 replay
  两侧口径一致，无重复计数。
- **TUI 状态**（`chat_types.rs` / `chat.rs`）：`ChatView.tokens_total: u64`；
  `apply` 对 `LlmUsage` 做 `saturating_add`。子代理事件经既有 `SubagentChild`
  路由自动进子视图，主视图不合并子 session——聚焦子代理时显示该子 session 累计。
- **replay**（`session_ui/replay.rs`）：`replay_one` 对 assistant 消息求和
  `usage.total_tokens`（覆盖 `replay_into_chat` / `replay_messages` /
  `reconstruct_child_view` 全路径）；`replay_into_chat` 新增
  `preserve_tokens_total` 参数并在内部 `max()`——`TranscriptReset`（压缩）重建
  时消息列表被截断、求和只会变小，用 live 累计值兜底防回退。原 app_loop 内联的
  重建块提取为 `session_ui::rebuild_after_reset`（app_loop.rs 800 行红线以下）。
- **调用点**：`app_loop.rs` TranscriptReset 重建传 live `tokens_total`；
  `app_helpers.rs`（冷启动 resume）传 0；`app_task.rs` 切换会话仅当
  `new_session_id == *session_id`（回到同一 session）时兜底，跨 session 不泄漏
  旧会话成本。
- **格式化**（`fmt.rs`）：纯函数 `format_tokens_cost_m(total)`：
  `m = total/1e6`，`total == 0` 显示 `0`，`>0` 时以 0.001 为下限，`{:.3}m`。
- **渲染**（`theme.rs` / `render.rs`）：`theme::rounded_block_line_tok(title,
  tokens_total, area_w, turn_ms)` 在 `rounded_block_line` 基础上加底边框左下
  title（`[tok cost …]`，`turn_ms > 0` 时追加 `· [turn cost …]` 段，默认前景
  色）。窄宽度分级丢弃：tok 段自身放不下（含右下指示 ~12 列保留）时整体丢弃
  label；仅 turn 段溢出时只留 tok 段。`render.rs` 调用点传入既有 `tail_ms`。
  原 `[turn cost]` 独占的内容尾行（含 1 行高度预留）随之移除——计时器唯一
  归宿是底边框角，不再双重显示。copy 模式在 block 构造前 early-return，
  天然不泄漏。
- **idle 刷新修复**（`app.rs`）：`LlmUsage` 是纯展示事件，turn 结束进入
  idle 后无任何输入重脏化显示缓存，角标曾滞后一轮。idle 边界（`Proceed`
  分支）补 `body_refresh_pending = true;` 单行修复，app.rs 保持 800 行红线。
- **透传面**：web（`sse_kind`/`sse_data` 泛化广播 + 持久化）、client
  （`from_sse` 解码）零改动即兼容；新 kind 对同版本无影响，旧 client 遇未知
  kind 走 `from_sse` 的 `None` 默认分支。`cli/display.rs` 穷尽 match 补
  忽略臂。

## 对齐口径

1. m = 1,000,000 tokens；空 session（0）显示 `0`；不足 0.001m 的正值显示 `0.001m`。
2. 累计 = provider 上报的 `usage.total_tokens`（input+output 含 cache），每条
   assistant 消息计一次；非本地估算。
3. 主视图显示主 session 累计；聚焦 subagent 显示该子 session 累计（不合并）。
4. 常显（不随 `[turn cost]` 消失）；边框角标两段均为默认前景色。

## 已知近似

- provider 不返回 usage 的轮次不计入（显示为真实下限口径）。
- 压缩折叠丢历史 usage → replay 求和变小，以 `max(live 累计)` 兜底（近似保留）；
  跨进程重启后（`--continue`）若历史上发生过压缩，压缩前部分不可恢复。

## 测试覆盖（rules/01 / 03）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 格式化：0/下限/进位/表驱动 | `tok_cost_zero_and_floor_at_one_thousandth_million`、`tok_cost_scales_in_millions` | `crates/tui/src/fmt.rs` |
| LlmUsage 事件累加 | `llm_usage_events_accumulate` | `crates/tui/src/chat_tests/tok_cost.rs` |
| 累计 display-only + 跨 TranscriptReset 保留 | `llm_usage_accumulation_is_display_only_and_survives_reset` | 同上 |
| 子代理 usage 只进子视图 | `subagent_child_usage_accumulates_into_child_view_only` | 同上 |
| replay 消息求和 | `replay_sums_persisted_assistant_usage` | 同上 |
| 底边框左下角空 session 显示 0 | `tok_cost_corner_defaults_to_floor_on_bottom_border` | `crates/tui/src/render_tests/tok_cost.rs` |
| 注入累计值 1.235m（三位小数） | `tok_cost_corner_shows_accumulated_total` | 同上 |
| 与右下跟随指示共存不重叠 | `tok_cost_coexists_with_right_bottom_indicator` | 同上 |
| copy 模式隐藏 | `tok_cost_hidden_in_copy_mode` | 同上 |
| 窄宽度丢弃防碰撞 | `tok_cost_dropped_on_narrow_width` | 同上 |
| 计时中角标追加 `· [turn cost]` 段 | `tok_cost_border_appends_turn_cost_segment_when_timing` | 同上 |
| idle 边界角标不再滞后一轮（usage 无输入重脏化） | `turn_final_usage_batch_is_idle_and_paint_eligible` | `crates/tui/src/app_loop_tests/tok_cost_idle_refresh_tests.rs` |
| 计时器随角标显示/隐藏、永不与内容同行（迁移回归） | `body_shows_turn_cost_timer_at_content_tail`、`body_turn_cost_timer_always_own_line` 等 | `crates/tui/src/render_tests/timer.rs` |
| 分级丢弃：先弃 turn 段保 tok 段 | `tok_cost_border_drops_turn_segment_before_tok_on_narrow_width` | 同上 |
| store replay 求和（integration） | `replay_sums_real_usage_from_persisted_messages` | `crates/tui/tests/tok_cost_replay.rs` |
| TranscriptReset 重建保累计（integration） | `preserve_floor_keeps_lifetime_total_across_transcript_reset` | 同上 |
| 子视图事件路径累计（integration） | `child_view_replay_accumulates_llm_usage_events` | 同上 |
| 发射时机：persist 后、RoundEnd 前，恰一条 | `usage_round_emits_llm_usage_between_persist_and_round_end` | `crates/session/tests/llm_round_lifecycle.rs` |
| 无 usage 轮次不发射 | `usage_less_rounds_emit_no_llm_usage` | 同上 |
| SSE kind 全变体 roundtrip（22 kinds） | `from_sse_roundtrips_all_variants` | `crates/session/src/runner/event.rs` |

## 回归（rules/02 门）

- 全量：`cargo test --workspace --no-fail-fast` → 201 个 result 汇总 /
  **3185 passed / 0 failed**（含并发迭代全部用例；本轮含边框 `· [turn cost]`
  段 2 个新测试与三分位格式化同步；数字为 E0063 修复后当次实跑——含并发
  `LlmUsage` 字段扩展、file-mention 与 subagent 折叠用例）
- clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings`
  复跑 → 零警告（含本次 render.rs / timer.rs / 新增测试文件）
- fmt：本次改动文件全部 rustfmt（edition 2021）清洁；工作区其余文件存在
  并发迭代的存量未格式化项，未触碰

## 关联

- 事件发射语义与 `[turn cost]` 的 round 生命周期对齐
  （`features/changelog/2026-08-20/stream-retry-deterministic-chunk-error.md`
  同批回归门数字见上）。
