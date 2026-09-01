Commit: (working-tree, sidecar 收敛落库)

# TUI `/sidecar` 问询 actor：主任务旁路快照问答，零持久化、成本记主任务

## 背景
主任务运行中常需针对当前会话上下文提问（解释某段输出、核对计划语义），这类问答不应挤占主循环，也不应污染主 transcript 与持久化面。本变更新增 `/sidecar <question>`：一条伴随主会话的临时 agent 循环，快照主任务上下文作答，问答内容只进 TUI 侧车盒。

## 变更
### session 侧 —— sidecar 循环运行时
- **`crates/session/src/runner/sidecar.rs`**（新增，296 行）：`SidecarConv` + `run_sidecar_turn`/`new_conv_from`。契约三条：
  - **Snapshot-in**：子循环从父 transcript 克隆（或调用方快照）起步，follow-up 复用同一 `SidecarConv` 内存 Q/A 历史；
  - **零持久化**：子循环永不 `.with_store()`，`Sidecar*` 帧经 `SessionEvent::is_sidecar_frame` 被 `EventSink::push` 丢弃（`crates/session/src/event_sink.rs`）；
  - **成本记主任务**：子循环每轮 `LlmUsage` 以裸事件（不包 `SidecarChild`）转发，下游按主任务轮次累计并持久化（web replay 对账）。
- **`crates/session/src/runner/event.rs`**：新增 `SidecarStart`/`SidecarChild`/`SidecarTurn` 三帧（含序列化与帧分类）。
- **`crates/tui/src/worker.rs`**：sidecar turn 完成后仅持久化裸 `LlmUsage`，内容帧只进 UI 事件流。

### TUI 侧 —— 侧车盒与焦点语义
- **`crates/tui/src/sidecar_ui.rs`**（新增，141 行）：每会话常驻 sidecar actor 任务；`/task` 切换即弃旧 sender，历史绝不跨会话；问题走有界 channel `try_send`，主循环永不阻塞、不经 steer/queue/prompt。
- **`crates/tui/src/chat_sidecar.rs`**（新增，220 行）：`fold_sidecar` 三帧折叠（Start 推块并自动聚焦、Child 流入嵌套 view、Turn 定格状态）；`focused()` 投影给 `compute_display`。
- **`crates/tui/src/app_loop.rs`**（`compute_display`）：聚焦侧车时 body 换嵌套 view、mode chip 读 `sidecar`、**ctx 表读 Turn 帧累计 tokens**——嵌套 view 自身 `context_used` 恒 0 属设计语义（子 usage 裸转发，块头 Turn 摘要是唯一诚实 per-box 数字）；主任务 running 态不受影响。
- **`crates/tui/src/app_submit.rs`**（新增，165 行）：Submit 臂机械抽取（行为不变），承载 `/sidecar` 前缀路由。
- **`crates/cli/src/display.rs`**：headless 输出对 `Sidecar*` 帧刻意静默（侧车 UX 归 TUI，不发明渲染）。

### 同提交收敛项
- plan `--prompt-file` 不再在提示词层注入 build subagent 广告（存储态剥除 + 装配态 kind 感知兜底 + hide 谓词收敛 core），e2e 接缝 `crates/cli/tests/prompt_file_run_assignment.rs`（新增）。详见 [plan-prompt-file-no-build-ad](../2026-09-01/plan-prompt-file-no-build-ad.md)。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 侧车回合不持久化 / follow-up 见前轮 / 控制问题拒绝 | `sidecar_turn_answers_without_persisting` 等 | `crates/session/tests/sidecar_loop.rs` |
| 事件门：`Sidecar*` 帧不落库、裸 `LlmUsage` 落库 | `event_sink_filters_sidecar_frames`、`worker::tests_sidecar::*` | `crates/session/tests/sidecar_loop.rs`、`crates/tui/src/worker/tests_sidecar.rs` |
| 折叠：推块/聚焦/嵌套流/未知 id 吞弃/成本不双计 | `sidecar_start_pushes_block_and_auto_focuses` 等 | `crates/tui/src/chat_tests/sidecar_fold.rs` |
| 显示：聚焦 ctx 取 Turn 累计、失焦还原父 body | `focused_sidecar_ctx_is_scoped_to_the_sidecar_view` 等 | `crates/tui/src/app_loop_tests/sidecar_display_tests.rs` |
| 路由：bare 入侧车/follow-up/running 不扰主任务/lookalike 不截获 | `idle_sidecar_question_routes_to_the_sidecar_actor` 等 | `crates/tui/src/key_handler_sidecar_tests.rs` |
| actor 端到端：follow-up 零内容持久化 + usage 落库 | `sidecar_actor_answers_follow_ups_without_persisting_content` | `crates/tui/src/sidecar_ui_tests.rs` |

## 全量回归
- `cargo test --workspace --no-fail-fast` → 132 个目标全绿；唯一红
  `nodes_smoke_proc` 为环境性（daemon 冷启动 107s > smoke 90s 就绪预算，
  手工复刻 107s 可达消歧转绿，见
  [plan-prompt-file-no-build-ad](../2026-09-01/plan-prompt-file-no-build-ad.md)）。
- `cargo test -p opencoder-tui --lib sidecar` → 25 passed / 0 failed
  （clippy 修复后当次树复验）。
- clippy：`-D warnings` 全 workspace 零警告——侧车新面修复 3 处
  （`flatten_sidecar` 参数过多加 allow、`bool_assert_comparison` 改
  `assert!`、`len_zero` 改 `is_empty`）。
- build：`cargo build --workspace` 干净（12m43s）。
- 行数：sidecar 新文件全部 ≤400（最大 `sidecar_loop.rs` 320）。

## Impact Surface
- 用户可感知：TUI 新增 `/sidecar <question>`；主任务 token 成本含侧车开销；侧车问答不出现在持久 transcript / web 回放。
- 不影响：CLI headless 输出面（静默）、store schema（零新表零新列，sidecar 帧不落库）、web API 面。

## Related Docs
- [agents/session](../../../agents/session/index.md)
- [agents/tui](../../../agents/tui/index.md)
- [既有相关 changelog](../2026-09-01/plan-prompt-file-no-build-ad.md)
