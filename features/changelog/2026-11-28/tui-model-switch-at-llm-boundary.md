# TUI：`/model` 去除 Shared 语义；运行中切换在当前 LLM 请求完成后生效

一句话：`/model` 的菜单 scope、切换 patch 与会话覆盖写入只剩 Act/Plan（顶层 `model` 即 Shared
默认只由 onboarding / `/config` 写入）；任务运行中 `/model` 选择不再等整个 turn 结束——runner 在
**当前这次 LLM 请求完成后**立即应用，并通过 display-only 的 `ModelOverrideApplied` 让 UI 在同一批
折叠里提交显示，左上角模型 = 下一次请求实际使用的模型。取代同日早先的
[tui-model-switch-while-running.md](tui-model-switch-while-running.md)（turn 边界 + worker 命令）。

## 背景与决策

- 早先方案把运行中的选择经 worker 串行命令通道（`ApplySessionDeferred`）排队到 turn 结束。一个 turn
  内通常有多轮 LLM 请求（工具执行之间），用户希望换模型后**下一次请求**就用新模型。
- 决策：
  - session 侧新增 `ModelSwitchQueue`（`Arc<Mutex<VecDeque<(ConfigMode, String)>>>`，纯共享状态），
    句柄随 `SessionState` 创建、经 `ChildRuntimeHandles.model_switches` 交给 UI；push 永不阻塞，
    不经过 worker 命令通道，不改 `start_worker` / `rebind_session`。
  - runner 钩子 `apply_queued_model_switches`：`run_one_llm_call` 返回 Ok 后（cancel 检查前）与 Err
    分支（`LlmRoundEnd` 后）各一处，Codex harness `run_turn` 后一处。每项：持久化覆盖 →
    `take_model_switch` 有变化发 `ModelSwitch` → 发 `ModelOverrideApplied`（失败携带 error，不中断
    后续项）。事件顺序：`LlmRoundStart` → deltas → `ModelSwitch` → `ModelOverrideApplied` → 工具 →
    `LlmRoundEnd`。
  - 新事件 `SessionEvent::ModelOverrideApplied { mode: "act"|"plan", model, error }`：`mode` 为 wire
    key（`ConfigMode` 不实现 serde）；`is_ephemeral_frame() = is_sidecar_frame() || 本事件`，
    `event_sink` / `runc_events` / `agent-step-runner` / TUI `persist_event` 的持久化门统一改用
    `is_ephemeral_frame`，事件只用于显示、不落盘。
  - UI：`queue_session_switch(json, queue, chat)` 只接受 Act/Plan 纯模型 patch；`fold_ui_events`
    在 `UiEvent::Session` 分支对 `ModelOverrideApplied` 调 `commit_model_override`（wire key 不是
    act/plan 只报告不提交），再走原 `chat.apply`。删除 `UiCmd::ApplySessionDeferred` /
    `UiEvent::ModelCommit`。
  - `/model` 去 Shared：`ProviderList::new`、`ProviderForm::from_existing` / `new_blank` 默认 Act，
    列表 `n` 新增表单沿用列表 scope；`switch_mode_provider_json(Shared)` 归一为 `modes.act`，删除
    无调用方的 `switch_provider_json`；`selection_from_patch` 不再把顶层 `model` 当作会话选择；
    `to_json_for_mode(Shared)` 仅保留给 onboarding（它要写配置文件默认）。
- 不改：`SessionState` legacy 行首次写入把 Shared/未选模式钉到 `config.model` 的语义（显示副本
  `display_overrides` 镜像）、`commit_display`、标题逻辑、远端会话 `/model` 拒绝、Web 接口。

## 标记文案

- 排队：`[/model] {Act|Plan} queued → {model} (applies when the current LLM round ends)`
- 应用：`[/model] {Act|Plan} switched (session only) → {model}`（与空闲路径一致）
- 失败：`[/model] session-only change failed: {error}; runtime unchanged`
- 非 Act/Plan wire key：`[/model] ignored a model pick outside Act/Plan (`{key}`)`
- 无选择（运行中）：`[/model] session-only change contained no model selection`
- 无选择且无 Provider（空闲）：`[/model] session-only change contained neither Provider data nor a model selection`（不发 worker 命令）
- Provider 拒绝：`[/model] busy — Provider changes apply when idle; model selection is unchanged`

## 实现

- `crates/session/src/model_switch.rs`（新建）、`model_switch_tests.rs`：队列与 runner 钩子。
- `crates/session/src/lib.rs`、`resume.rs`：`SessionState.model_switch_queue`。
- `crates/session/src/runner/event.rs`：`ModelOverrideApplied` 变体、`sse_kind="model_override_applied"`、
  `sse_data` / `from_sse`、`coarse_kind=Step`、`is_ephemeral_frame`。
- `crates/session/src/runner/mod.rs`：三处 `apply_queued_model_switches` 钩子。
- `crates/session/src/event_sink.rs`、`crates/dag-runtime/src/exec/runc_events.rs`、
  `crates/dag-runtime/examples/agent-step-runner.rs`、`crates/tui/src/worker.rs::persist_event`：
  持久化门改 `is_ephemeral_frame()`。
- `crates/tui/src/worker.rs`：删 `ApplySessionDeferred` / `ModelCommit`；`ChildRuntimeHandles.model_switches`。
- `crates/tui/src/model_session_switch.rs`：`queue_session_switch(json, queue, chat)`、
  `commit_model_override`、`commit_session_switch(chat, config, mode, model, result)`。
- `crates/tui/src/app_loop.rs`：`ModelOverrideApplied` 折叠；删 `ModelCommit` arm。
- `crates/tui/src/app_loop_model.rs`、`app_popups.rs`、`app.rs`：透传 `ModelSwitchQueue`。
- `crates/tui/src/chat.rs`、`crates/local/src/display.rs`：`ModelOverrideApplied` 穷举 arm。
- `crates/tui/src/model_scope.rs`、`model_menu/{patch,list,provider_form}.rs`：`/model` 去 Shared。
- `crates/tui/src/model_menu/tests/`：`provider_tests.rs` 原已 800 行，按职责拆为
  `provider_patch_tests.rs`（patch 序列化）/ `provider_tests.rs`（列表）/ `provider_form_tests.rs`（表单编辑）/
  `provider_form_cursor_tests.rs`（光标渲染）；测试名不变。
- `agents/frontend/tui/index.md`、`agents/runtime/session/index.md` 同步说明。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 队列 push 顺序 / drain 清空、`mode_key` 映射 | `queue_keeps_submission_order_and_drains_once`、`mode_keys_roundtrip` | `crates/session/src/model_switch_tests.rs` |
| 空队列应用无事件 | `no_pending_pick_emits_nothing` | `crates/session/src/model_switch_tests.rs` |
| 应用后事件顺序 `ModelSwitch` → `ModelOverrideApplied`；他模式 pick 不发 `ModelSwitch` | `applied_pick_switches_the_effective_model_and_reports_its_mode`、`pick_for_the_other_mode_reports_without_a_model_switch` | `crates/session/src/model_switch_tests.rs` |
| 门控流：请求进行中 push 的 pick 在该请求后、下一请求前生效；`requests[0/1].model`；事件顺序；assistant 消息记录的 model | `pick_queued_during_a_request_applies_after_it_and_before_the_next_one` | `crates/session/tests/model_switch_mid_turn.rs` |
| Store 拒绝：`ModelOverrideApplied{error}` 无 `ModelSwitch`，运行时模型不变；恢复后再 push 成功落盘 | `store_failure_reports_the_pick_and_keeps_the_runtime_model` | `crates/session/tests/model_switch_mid_turn.rs` |
| SSE roundtrip 计数；`is_ephemeral_frame` 覆盖 sidecar 与 pick 通知 | `from_sse_roundtrips_all_variants`、`is_ephemeral_frame_gates_sidecar_content_and_model_pick_notifications` | `crates/session/src/runner/event/tests.rs` |
| 运行中 `Enter`+`n`：无 worker 命令、队列得到 `(Act, model)`、外层 config / 显示 / client 不变、queued 标记；空闲同按键发 `ApplySession(Act)` 且不动队列 | `session_only_switch_while_running_is_queued_without_worker_command` | `crates/tui/src/app_loop_tests/config_busy_gate_tests.rs` |
| 运行中排队 → `ModelOverrideApplied` 折叠后提交 Act 覆盖、Plan 钉基值、Shared 不变、`chat.model` 跟随、switched 标记；折叠不发命令、不改 running | `queued_selection_commits_display_only_after_model_override_applied` | `crates/tui/src/app_loop_tests/model_deferred_switch_tests.rs` |
| Plan pick 只动 Plan 槽，Act 会话显示不变 | `plan_pick_only_moves_the_plan_slot` | `crates/tui/src/app_loop_tests/model_deferred_switch_tests.rs` |
| 失败 pick：显示保持旧模型、失败标记 | `refused_pick_keeps_display_on_previous_model` | `crates/tui/src/app_loop_tests/model_deferred_switch_tests.rs` |
| `shared` / 未知 wire key 只报告不提交 | `pick_outside_act_and_plan_is_reported_not_committed` | `crates/tui/src/app_loop_tests/model_deferred_switch_tests.rs` |
| Provider patch 运行中在排队前拒绝 | `provider_patch_while_running_is_refused_before_queueing` | `crates/tui/src/app_loop_tests/model_deferred_switch_tests.rs` |
| 顶层 `model` / 无模型 patch 不排队 | `patch_without_act_or_plan_model_is_refused` | `crates/tui/src/app_loop_tests/model_deferred_switch_tests.rs` |
| 多个 pick 保持提交顺序、不合并、排队不提交显示 | `queued_picks_keep_submission_order` | `crates/tui/src/app_loop_tests/model_deferred_switch_tests.rs` |
| Act 选择的显示终态与空闲路径一致 | `commit_display_matches_the_idle_switch_end_state` | `crates/tui/src/app_loop_tests/model_deferred_switch_tests.rs` |
| 空闲 session-only 切换写 Act 覆盖、Shared 不变、不落盘 | `handle_model_outcome_session_only_skips_disk_write` | `crates/tui/src/app_loop_session_only_tests.rs` |
| `selection_from_patch` 只读 Act/Plan | `patch_selection_reads_only_act_and_plan` | `crates/tui/src/model_scope.rs` |
| Plan 选择不动 Shared 槽；顶层 `model` 无选择 | `plan_selection_keeps_the_shared_overlay_without_rebuilding_client` | `crates/tui/src/model_session_switch_tests.rs` |
| Provider + Act 选择标记、乱序 ack 不交叉提交 | `provider_add_replaces_fields_and_rebuilds_only_after_ack`、`out_of_order_completions_do_not_cross_commit_or_rollback` | `crates/tui/src/model_session_switch_tests.rs` |
| `switch_mode_provider_json(Shared)` 归一 Act、Plan 写 `modes.plan`，均无顶层 `model` | `delete_and_switch_patches` | `crates/tui/src/model_menu/tests/provider_patch_tests.rs` |
| 列表 Enter/y/n/Enter 产出 `modes.act.model` 且无顶层 `model` | `list_enter_then_y_saves_as_default`、`list_enter_then_n_is_session_only`、`list_enter_then_enter_saves_as_default` | `crates/tui/src/model_menu/tests/provider_tests.rs` |
| scoped 编辑表单保存只改 provider、不选模型（无顶层 `model`、无 `modes`） | `provider_form_save_produces_patch` | `crates/tui/src/model_menu/tests/provider_form_tests.rs` |
| 新增表单沿用列表 Act scope | `add_form_n_returns_session_only_patch_and_enter_preserves_disk_save` | `crates/tui/src/model_menu/tests/session_only_ui_tests.rs` |
| `/model` 运行中派发打开菜单、无 UiCmd（沿用） | `slash_action_model_running_opens_menu_without_worker_command` | `crates/tui/src/app_loop_slash_action_tests.rs` |

## 回归记录（2026-11-28，最终代码）

- `cargo test -p opencoder-session --lib -- model_switch event::` → 18 passed / 0 failed
- `cargo test -p opencoder-session --test model_switch_mid_turn --test mode_model_requests --test llm_round_lifecycle` → 2 + 5 + 4 passed / 0 failed（Clippy 调整 `position` 辅助函数后单独重跑 `--test model_switch_mid_turn` → 2 passed）
- `cargo test -p opencoder-dag-runtime --lib runc_events` → 2 passed / 0 failed
- `cargo test -p opencoder-tui --lib --no-run` 后直接运行测试二进制（`target/debug/build/opencoder-tui/57569e9a6c19ed40/out/opencoder_tui-57569e9a6c19ed40`）：
  - `app_loop keymap model_session_switch model_scope model_menu worker chat chat_display chat_sidecar session_only slash_action switch_gate dispatch_cmd_tests app_tests app_helpers app_subtask onboarding remote` → 811 passed / 0 failed（测试文件拆分后重跑仍为 811，测试名集合与拆分前一致）
  - `app::app_task cli_model_precedes key_handler::running_mode_tests app::app_switch subtask_events`（`rebind_session` 链路）→ 53 passed / 0 failed
- `cargo check -p opencoder-local`（非 test）→ 通过；其 test 目标里 `crates/local/src/todos_interrupt/tests/command.rs` 的既有 `unresolved import opencoder_database::writer_lock` 与本次无关，未处理。
- Clippy：`cargo clippy -p opencoder-tui -p opencoder-session -p opencoder-dag-runtime --tests -- -D warnings` → 零警告；`cargo clippy -p opencoder-local -- -D warnings` → 零警告。
- `cargo fmt -p opencoder-tui -p opencoder-session -p opencoder-dag-runtime -p opencoder-local --check` → 通过。
- 测试范围：本次修改及其直接影响的链路；未运行全量测试，未运行 runc 沙箱端到端（dag-runtime 改动仅为事件持久化门一行，不涉及节点/工作区/步骤能力）。

## 遗留说明

- Web `POST /sessions/:id/model` 在 drain 运行中仍 409，可后续接入同一个 `ModelSwitchQueue`。
- 远端会话的 `/model` 仍在派发前被 `chat.remote` 规则拒绝，未改。
