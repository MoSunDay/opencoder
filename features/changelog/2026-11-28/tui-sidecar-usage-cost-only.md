# TUI：`/sidecar` 用量改为带标签的 `SidecarUsage`——只进主任务 `[tok cost]`，不再顶掉主任务 ctx

一句话：`/sidecar` 提问后主任务状态栏的 `ctx` 跳到了侧车的窗口大小。根因是侧车每轮
`LlmUsage` 以**裸**事件转发进主 `ChatView::apply`，而该臂同时写 `tokens_total`（成本，
符合预期）和 `real_context_tokens`（ctx 表，不该动）。本轮新增带 sidecar id 标签的
`SessionEvent::SidecarUsage`：成本照常记主任务并落库，ctx 表只认主任务自己的 `LlmUsage`。

## 背景与决策

- 现象：主任务 ctx 为 N，`/sidecar` 问一句（子循环带着主 transcript 快照 + 侧车系统提示 +
  问答），主任务状态栏 `ctx` 变成侧车那一轮的 `total_tokens`（通常更大）。
- 根因：`runner/sidecar.rs` 把子循环的 `LlmUsage` 以裸事件转发（设计意图是「成本记主任务」），
  但裸事件没有来源信息，`ChatView::apply` 的 `LlmUsage` 臂无法区分主任务轮次与侧车轮次，
  一并写入了 `real_context_tokens`——这与该字段既有契约相悖（子代理轮次「不写入父的该字段」，
  侧车更是借用快照、从不回写主 transcript）。
- 决策（对齐 `SubagentChild(LlmUsage)` 的既有先例——父只累成本、不写 ctx）：
  - session 侧新增 `SessionEvent::SidecarUsage { id, total_tokens, input_tokens, output_tokens }`，
    `run_sidecar_turn` 转发它替代裸 `LlmUsage`；`SidecarChild` 内仍绝不包 usage。
  - 它是**成本通道而非内容帧**：不纳入 `is_sidecar_frame`，`EventSink::push` /
    `worker::persist_event` 照常落库；SSE kind 为 `sidecar_usage`（不再借用 `llm_usage`，
    否则 web `/events` 回放会把它当成主任务最近一轮用量顶掉页脚），`from_sse` 可逆映射，
    coarse kind 为 `Step`。
  - TUI `ChatView::apply` 新增 `SidecarUsage` 臂：只 `tokens_total += total_tokens`，
    `real_context_tokens` 不动。侧车面板自身的数字仍来自 `SidecarTurn.total_tokens`。
- 不做：不改历史已落库的裸 `llm_usage` 行（TUI 恢复只回放 messages，不受影响；web 回放
  只影响页脚「最近一轮」显示，直到下一次快照刷新）。`[tok cost]` 在重启后的对账缺口
  （主会话 `session_events` 不回放）为既有问题，本轮不扩展。

## 变更

- `crates/session/src/runner/event.rs`：新增 `SidecarUsage` 变体；`sse_kind`/`sse_data`/
  `from_sse`/`coarse_kind` 补齐；`is_sidecar_frame` 文档注明成本通道刻意不纳入。
- `crates/session/src/runner/sidecar.rs`：`run_sidecar_turn` 的 usage 转发改为
  `SidecarUsage { id: conv.id, .. }`；模块/`SidecarTurn` 文档同步。
- `crates/session/src/event_sink.rs`、`crates/session/src/lib.rs`、`crates/local/src/display.rs`、
  `crates/tui/src/worker.rs`、`crates/tui/src/sidecar_ui.rs`（收集/落库 `SidecarUsage`）、
  `crates/tui/src/chat_sidecar.rs`、`crates/tui/src/chat_types.rs`：契约注释与匹配臂同步。
- `crates/tui/src/chat.rs`：新增 cost-only 臂。
- `crates/tui/src/app_loop_tests/sidecar_usage_tests.rs`（新）、`app_loop_tests/mod.rs`：
  app-loop `fold_ui_events` 批路径对 `SidecarUsage` 的折叠用例。
- `crates/tui/src/remote/tests/protocol.rs`：远端 TUI 回放路径（`from_sse("sidecar_usage")` →
  `transcript::replay` → `ChatView::apply`）用例。
- `crates/web/spa/src/reduce.js`：SPA reducer 显式 `case 'sidecar_usage'` no-op（原先落 `default`）；
  `reduce.test.js` 补用例。
- `crates/web/tests/replay_fidelity.rs`：`all_variants()` 纳入 `SidecarUsage`（20 → 21）。
- `agents/runtime/session/index.md`：runner 索引补 sidecar 成本/ctx 契约一句。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 子循环每轮 usage 以带 id 的 `SidecarUsage` 转发、绝不裸 `LlmUsage`、`SidecarChild` 不包 usage | `sidecar_turn_answers_without_persisting` | `crates/session/tests/sidecar_loop.rs` |
| `EventSink::push` 丢弃内容帧、`SidecarUsage` 以 `sidecar_usage` 落库 | `event_sink_filters_sidecar_frames` | `crates/session/tests/sidecar_loop.rs` |
| 多轮 turn（工具轮 → 回答轮）：每轮各转发一条带 id 的 `SidecarUsage`，`SidecarTurn.total_tokens` 为其和、`rounds` 为轮数 | `sidecar_multi_round_turn_tags_every_round_and_sums` | `crates/session/tests/sidecar_loop.rs` |
| `sse_kind`/`sse_data`/`from_sse` 往返覆盖 `SidecarUsage` | `from_sse_roundtrips_all_variants` | `crates/session/src/runner/event/tests.rs` |
| 旧负载（仅 `id`+`total_tokens`）可反序列化、kind/coarse 正确、不被持久化门拦 | `sidecar_usage_old_payload_defaults_split_fields_to_zero` | `crates/session/src/runner/event/tests.rs` |
| 持久化门只拦三个内容帧，`SidecarUsage`/裸 `LlmUsage`/`Subagent*` 放行 | `is_sidecar_frame_marks_exactly_the_sidecar_variants` | `crates/session/src/runner/event/tests.rs` |
| 父视图：`SidecarUsage` 累 `tokens_total`、`real_context_tokens` 不动 | `sidecar_usage_accounts_cost_without_moving_the_parent_ctx`、`sidecar_usage_before_any_parent_round_keeps_ctx_unknown` | `crates/tui/src/chat_tests/sidecar_fold.rs` |
| 包在 `SidecarChild` 里的 usage 不双计、不动 ctx | `sidecar_child_llm_usage_is_not_double_counted`、`sidecar_child_wrapped_sidecar_usage_is_dropped`（嵌套视图的成本/ctx 也不动） | `crates/tui/src/chat_tests/sidecar_fold.rs` |
| app-loop 批路径（`fold_ui_events`）：`SidecarUsage` 到达 `ChatView::apply`，成本进主任务、ctx 不动、不是 idle 边界、不跳帧 | `sidecar_usage_batch_adds_cost_and_keeps_the_parent_ctx`、`sidecar_usage_batch_before_any_parent_round_keeps_ctx_unknown` | `crates/tui/src/app_loop_tests/sidecar_usage_tests.rs` |
| 远端 TUI 回放：持久化 kind `sidecar_usage` 经 `from_sse` 解码后 cost-only 折叠（含无 split 字段的旧负载） | `remote_replay_sidecar_usage_adds_cost_without_moving_ctx`、`remote_replay_sidecar_usage_without_split_fields_still_adds_cost` | `crates/tui/src/remote/tests/protocol.rs` |
| 状态栏 `ctx` 渲染保持主任务自己的轮次，不显示侧车窗口 | `sidecar_usage_does_not_move_the_status_bar_ctx` | `crates/tui/src/render_tests/status_ctx.rs` |
| `persist_event` 对 `SidecarUsage` 不设门，落库 kind 为 `sidecar_usage` | `sidecar_usage_is_persisted_under_its_own_kind` | `crates/tui/src/worker/tests_sidecar.rs` |
| actor 端到端：两轮问答只落两条 `sidecar_usage`，无内容帧、无裸 `llm_usage` | `sidecar_actor_answers_follow_ups_without_persisting_content`（及 Reset 系列用例） | `crates/tui/src/sidecar_ui_tests.rs` |
| web 回放：`sidecar_usage` 以同名 kind 往返 | `replay_kind_matches_live_kind_for_all_variants` | `crates/web/tests/replay_fidelity.rs` |
| web SPA reducer：`sidecar_usage` 不顶掉页脚 usage（主任务最近一轮 + 上下文 %），任何主轮次之前也保持 `null`；`nestedEventOf` 映射到同名 kind | `sidecar_usage is cost-only: it never replaces the footer usage (TUI chat.rs parity)`、`converts serde externally-tagged variants to SSE form` | `crates/web/spa/src/reduce.test.js` |
| 既有链路：裸 `LlmUsage` 仍写 ctx 与成本、子代理成本不写父 ctx、`[tok cost]` 渲染 | `real_context_tracks_latest_round_total_tokens`、`subagent_child_usage_accumulates_into_parent_and_child`、`tok_cost_corner_shows_accumulated_total` | `crates/tui/src/chat_tests/tok_cost.rs`、`crates/tui/src/render_tests/tok_cost.rs` |

定向回归（2026-11-28）：

- `cargo test -p opencoder-session --lib runner::event` → 12 passed / 0 failed
- `cargo test -p opencoder-session --test sidecar_loop` → 9 passed / 0 failed
- `cargo test -p opencoder-tui --lib sidecar` → 57 passed / 0 failed（含 `sidecar_fold`、
  `sidecar_display_tests`、`sidecar_usage_tests`、`sidecar_stream_isolation`、`sidecar_ui_tests`、
  `worker::tests_sidecar`、`key_handler::sidecar_tests`、
  `render::tests::status_ctx::sidecar_usage_does_not_move_the_status_bar_ctx`）
- `cargo test -p opencoder-tui --lib remote` → 47 passed / 0 failed（含两条
  `remote_replay_sidecar_usage_*`）
- web SPA：`npx vitest run src/reduce` → 4 files / 56 passed / 0 failed（`reduce.test.js` 46、
  `reduce.parity.test.js` 5 等；`sidecar_usage` no-op 用例在内）
- `cargo test -p opencoder-tui --lib tok_cost` → 22 passed；`--lib real_context` → 4 passed；
  `--lib status_ctx` → 7 passed；`--lib llm_usage` → 4 passed（既有 ctx/成本契约回归）
- `cargo test -p opencoder-tui --test tok_cost_replay` → 8 passed / 0 failed；
  `--test resume_context_used` → 7 passed / 0 failed
- `cargo test -p opencoder-web --test replay_fidelity` → 1 passed / 0 failed
- `cargo test -p opencoder-local display` → 12 passed / 0 failed
- Clippy：`cargo clippy --no-deps -p opencoder-session -p opencoder-tui -p opencoder-local
  -p opencoder-web --all-targets -- -D warnings` → 四个 crate 零警告（输出中唯一一条是工作区
  manifest 的 `unused workspace dependency opencoder-repo`，来自同日并行进行、与本变更无关的
  根 `Cargo.toml` 改动）
- `cargo fmt -p opencoder-session -p opencoder-tui -p opencoder-local -p opencoder-web -- --check` → 无差异
- 依赖 `SessionEvent` 的其余 crate 编译核对：`cargo check -p opencoder-server -p opencoder-agent
  -p opencoder-control -p opencoder-cli -p opencoder-worker -p opencoder-dag-runtime
  -p opencoder-project -p opencoder-node --all-targets` → 通过（新增变体未打破任何穷举匹配）
- 测试范围：本次修改及其直接影响的链路（session 事件/sidecar 循环、TUI 折叠/渲染/actor/
  持久化、web 回放保真）；未运行全量测试。
- 全量回归：`cargo test --workspace` **未运行**——提交经用户明示授权「免测」（同一工作树含并行
  会话的未提交 WIP，全量会混入他人半成品）；以上定向结果为实际执行并全部通过。
- 行数约束：新文件 `crates/tui/src/app_loop_tests/sidecar_usage_tests.rs` 144 ≤ 400；迭代文件
  最大为 `crates/web/spa/src/reduce.js` 792 ≤ 800（余量 8 行，下次改动前宜按职责拆分）、
  `crates/session/tests/sidecar_loop.rs` 561、`crates/tui/src/chat_tests/sidecar_fold.rs` 550、
  `crates/tui/src/remote/tests/protocol.rs` 446。

## Impact Surface

- 用户可感知：`/sidecar` 提问后主任务状态栏 `ctx` 不再变化；`[tok cost]` 仍含侧车开销。
- Web：`/events` 回放多出 `sidecar_usage` kind，SPA reducer 显式 no-op（`case 'sidecar_usage'`，
  不再把侧车用量当主任务最近一轮顶掉页脚）；远端 TUI `from_sse` 可映射并按 cost-only 折叠。
- Store：零 schema 变化（仍写 `session_events`，仅 `sse_kind` 值不同）。

## Related Docs

- `agents/runtime/session/index.md`（runner 索引：sidecar 成本/ctx 契约一句，已同步）
- `crates/web/spa/src/reduce.js`（SPA reducer：`sidecar_usage` 显式 no-op）
