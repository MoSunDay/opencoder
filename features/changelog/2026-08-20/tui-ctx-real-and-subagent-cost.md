# ctx 状态栏口径对齐 tok cost + 父视图 `[tok cost]` 计入子代理消耗

## 背景

两个口径偏差：

1. 状态栏 `ctx (used/limit)` 一直是本地估算（`context_used` + `sys_tokens`），
   与 provider 真实计费口径脱节——压缩触发的实际时点用户无法从 UI 预判。
2. `[tok cost]`（2026-08-20 `tui-session-tok-cost.md`）主视图只含父 session
   轮次，子代理（task 工具）的 LLM 消耗被折叠进子视图、父视图看不见——
   但用户为子代理轮次同样付费。

本次：`LlmUsage` 事件扩展 `input_tokens`/`output_tokens`（serde default 兼容），
状态栏在最近一个已完成轮次有真实数据时显示 `input+output`（不叠加
`sys_tokens`，provider 的 input 已含系统提示，避免双计）；父视图
`[tok cost]` live 与 replay 两条路径都累加子代理消耗，聚焦子视图仍只显示
子自身消耗。**语义变化：父视图数字从此包含子代理轮次**（此前不含）。

## 实现

- **事件扩展**（`crates/session/src/runner/event.rs`）：`LlmUsage` 增加
  `input_tokens`/`output_tokens`（`#[serde(default)]`，旧持久化行反序列化为
  0）；`sse_data` 带全字段，`from_sse` 对缺失字段取 0——旧 payload/旧 client
  全兼容。发射点（`runner/mod.rs`）从 `Usage` 原样透传三字段。
- **mod.rs 800 行红线**：`build_full_registry` 提取为 `runner/registry.rs`
  （mod.rs 822 → 795 行）。
- **TUI 真实 ctx**（`chat_types.rs` / `chat.rs`）：`ChatView.real_context_tokens:
  Option<u64>`——`LlmUsage` 时 = `input+output`（冻结在最近完成轮次，非累加）；
  `Compaction`/`TranscriptReset`/`ModelSwitch` 清空（上下文重写或 tokenizer
  变更）；子代理轮次**不**写入父的该字段（子上下文不属于父窗口）。
  `render_status.rs::resolve_ctx_used(real, est, sys)` 纯函数：`Some` 时用真实
  值且不加 `sys_tokens`，`None` 回退 `est + sys`（真实模式不显示 0）。
- **子代理 cost 汇入父视图（live）**（`chat.rs` `SubagentChild` 分支）：内层
  `LlmUsage` 除路由给子 view 外，同时 `tokens_total += total_tokens`。runner
  既有转发链（`subagent.rs` 把子事件包 `SubagentChild`）零改动即覆盖。
- **子代理 cost 汇入父视图（replay）**（`session_ui/replay.rs`）：
  `push_subagent_block` 在 push 前把 `view.tokens_total`（`reconstruct_child_view`
  事件路径 / 消息回退路径均已算好子总额）加进父 `tokens_total`。switch-back /
  TranscriptReset 的 `preserve_tokens_total` floor 取 `max`——两侧口径一致，
  天然不重复累加。子 session 记录缺失时只是少加（与现状一致），不报错。

## 对齐口径（更新）

1. ctx = 最近已完成 LLM 轮次的 provider 真实 `input+output`；无真实数据回退
   本地估算（不显示 0）。两轮之间冻结；真实/估算模式切换会跳变（已确认可接受）。
2. `[tok cost]` 仍为会话累计、永不回退；**现在含子代理消耗**。
3. 聚焦子代理视图时显示子自身 cost 与子自身真实 ctx（父视图含全部）。
4. `ModelSwitch` 清空父 real ctx（tokenizer 变更），cost 累计不受影响。

## 测试清单

| 断言 | 测试 | 位置 |
| --- | --- | --- |
| emit 携带 in/out 字段（persist 后、RoundEnd 前） | `usage_round_emits_llm_usage_between_persist_and_round_end` | `crates/session/tests/llm_round_lifecycle.rs` |
| 旧 payload 缺字段反序列化为 0（SSE + enum 双形态） | `llm_usage_old_payload_defaults_split_fields_to_zero` | `crates/session/src/runner/event.rs` |
| SSE roundtrip 含拆分字段 | `from_sse_roundtrips_all_variants` | 同上 |
| 真实 ctx 跟踪最近轮次 in+out（非累加） | `real_context_tracks_latest_round_input_plus_output` | `crates/tui/src/chat_tests/tok_cost.rs` |
| Compaction/TranscriptReset/ModelSwitch 清空 real ctx | `real_context_clears_on_compaction_transcript_reset_and_model_switch` | 同上 |
| ModelSwitch 清 real ctx 但 cost 不清 | `model_switch_clears_real_context_but_keeps_cost` | 同上 |
| 父含子（父 1_300 / 子 300），子上下文不写父 real ctx | `subagent_child_usage_accumulates_into_parent_and_child`（**翻转**自 child_view_only） | 同上 |
| ctx 解析：真实值胜出且不加 sys / 回退 est+sys | `resolve_ctx_used` 三测 | `crates/tui/src/render_tests/status_ctx.rs` |
| replay 父总额 = 父Σ + 子Σ（事件路径 + 消息回退路径） | `replay_folds_subagent_usage_into_parent_total_matching_live` | `crates/tui/tests/tok_cost_replay.rs` |
| replay 与 live 累加一致（floor 不双计） | 同上（live 对拍 + preserve floor 断言） | 同上 |

## 回归（rules/02 门）

- `cargo fmt --all -- --check` 清洁；`cargo clippy --workspace --all-targets`
  零警告。
- `cargo test --workspace --no-fail-fast` → 全绿（实跑数字见下）；首轮全量
  中 `queued_skill_drain::queued_skill_fires_at_consumption_not_during_kickoff`
  在并行满载下计时敏感 flake 一次，单测复跑通过，复跑全量无失败。

## 关联

- 前置迭代：`features/changelog/2026-08-20/tui-session-tok-cost.md`
  （`[tok cost]` 角标与 `LlmUsage` 事件首发，彼时主视图不含子代理）。
