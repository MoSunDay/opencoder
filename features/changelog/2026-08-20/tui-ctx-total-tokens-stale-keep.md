# ctx 口径统一为 LLM 返回的 `total_tokens`：估算 fallback 退出显示层，无真值时段沿用上一轮

## 背景

`[tok cost]` 已是 provider 真实计费口径，但状态栏 `ctx (used/limit)` 仍是
半估算混合体（2026-08-20 `tui-ctx-real-and-subagent-cost.md` 引入的
`input+output` + `None` 时回退 `chars/4` 估算），两处口径不一致：

1. **取值偏差**：ctx 用 `input_tokens + output_tokens`，而计费用
   `total_tokens`——两者在含 cache/reasoning token 的响应上不等，用户看到的
   ctx 与实际窗口占用脱节。
2. **回退噪声**：切模型 / 压缩 / TranscriptReset 后 `real_context_tokens`
   被清空，状态栏立刻跳回本地估算，下一轮真值返回又跳回——数字来回抖。
3. **resume 缺真值**：replay 重建只恢复 `tokens_total` 与估算
   `context_used`，`real_context_tokens` 恒为 `None`，恢复会话后状态栏永远
   显示估算直到新轮次完成。

本次：ctx 与 `[tok cost]` 统一为 **LLM 实际返回的 usage 口径**——ctx 取最近
一轮真实 `total_tokens`；估算 fallback（`context_used + sys_tokens`）彻底退出
显示层；无真值时段（切模型/压缩后到下一轮返回前）**沿用上一轮真值**（stale
但真实，用户已确认接受）；resume/replay 从持久化 `messages.usage` 重建真值。
session 侧 compaction 触发预算（功能逻辑）不受影响。

## 实现

- **live 跟踪**（`crates/tui/src/chat.rs`）：`LlmUsage` 时
  `real_context_tokens = Some(*total_tokens)`（原 `input+output`）；每轮覆盖
  语义不变。删除 `ModelSwitch` / `Compaction` / `TranscriptReset` 三处
  `= None` 清空——stale 保留至下一轮返回。
- **replay 重建**（`crates/tui/src/session_ui/replay.rs`）：`replay_one` 遍历
  assistant 消息时记录最近一条非零 `usage.total_tokens`（后到者胜），
  `replay_into_chat` / `replay_messages` / `rebuild_after_reset` 共用该路径；
  压缩截断重放段自然取截断后最近一条（空列表 → `None` → `—`）。
- **显示层**（`crates/tui/src/render_status.rs` + `render.rs`）：
  `resolve_ctx_used(real: Option<u64>) -> Option<u64>` 直通真值，删除
  `unwrap_or(context_used + sys_tokens)`；`render_status` 的 `used` 参数改
  `Option<u64>`——`None`（从未有过真值，如全新会话首轮返回前）ctx 显示
  `—`、thr 显示 0%。`ChatView.context_used` / `sys_tokens` 字段与累计逻辑
  保留（既有测试依赖，避免范围膨胀，后续另行清理）。
- 旧事件兼容：`LlmUsage` 的 `input/output` 本就 `#[serde(default)]`，旧
  payload 仅 `{"total_tokens": N}` 现在恰好给出正确 ctx（原 `input+output`
  语义下会算成 0）——行为变化属修正。

## 测试清单（rules/01）

| 语义 | 测试 | 位置 |
|---|---|---|
| ctx 取最近轮次 `total_tokens`（含 total≠in+out、旧 payload in/out=0） | `real_context_tracks_latest_round_total_tokens`（重写） | `crates/tui/src/chat_tests/tok_cost.rs` |
| Compaction/TranscriptReset/ModelSwitch 保留旧值（stale 沿用） | `real_context_survives_compaction_transcript_reset_and_model_switch`（翻转） | 同上 |
| ModelSwitch 保 real ctx 且 cost 不清 | `model_switch_keeps_real_context_and_cost`（重命名重写） | 同上 |
| replay 重建真值（最近非零 usage） | `replay_sums_persisted_assistant_usage`（扩断言） | 同上 |
| resolver：Some(t) 直用 / 无估算回退 | `resolve_ctx_used::{real_context_is_used_verbatim, no_estimate_fallback_without_real_data}`（重写） | `crates/tui/src/render_tests/status_ctx.rs` |
| 无真值渲染 `—` + 0%（thr meter 同步） | `status_bar_without_provider_truth_shows_placeholder_and_zero_percent`（新增） | 同上 |
| resume 从持久化 usage 重建真值 | `resume_rebuilds_real_context_from_persisted_usage`（新增） | `crates/tui/tests/resume_context_used.rs` |
| 压缩截断重放取尾部最近真值 | `resume_after_compaction_uses_surviving_tail_usage`（新增） | 同上 |
| total≠in+out 时 ctx 取 total | `replay_real_context_uses_total_tokens_not_input_plus_output`（新增） | `crates/tui/tests/tok_cost_replay.rs` |
| 旧事件 `{"total_tokens":42}` 兼容 | `old_usage_event_payload_with_only_total_rebuilds_real_context`（新增） | 同上 |
| 子代理父子 ctx 隔离（各自真值，子不写父） | `subagent_child_usage_accumulates_into_parent_and_child` / `e2e_mock_task_round_folds_child_usage_into_parent_view`（既有，语义不变） | 同上 / 同上 |

## 回归门（rules/02）

- `cargo fmt --all` 清洁；`cargo build -p opencoder-tui` 零警告。
- `cargo test -p opencoder-tui` → 1583 passed / 0 failed。
- `cargo test --workspace --no-fail-fast` → **3189 passed / 0 failed**
  （exit 0，全量实跑）。

## 已知取舍（用户确认）

- 压缩后到下一轮返回前 ctx/thr 短暂高估（沿用压缩前真值）。
- Anthropic 风格 `total_tokens` 不含 cache token，ctx 偏低——原 `in+out`
  口径同样不含，非新增风险。
- thr meter 在 stale 期沿用旧值；session 实际 compaction 触发逻辑独立不受
  影响。

## 关联

- 前置迭代：`features/changelog/2026-08-20/tui-ctx-real-and-subagent-cost.md`
  （`real_context_tokens` 首发与 `input+output` 口径，本条修正为 `total_tokens`
  并取消清空/回退）；`tui-session-tok-cost.md`（`[tok cost]` 首发）。
