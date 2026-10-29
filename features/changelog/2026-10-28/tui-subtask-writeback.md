# TUI 一次性 Server 子任务：提交、回写与恢复防回退

一句话：在 TUI 输入 `@<capability> <问题>` 即可向已登记的 Server Agent/Operator 发起一次性子任务——本地先落 `tui_remote_*` bookmark 会话并注册进主任务的 runtime registry，后台 watcher 轮询到终态后把结果摘要与 token 用量回写主任务，`[sub x/x]` 徽标与 `[tok cost]` 实时跟进，`/subtasks` 全屏视图提供刷新/取消/追加/进入对话，Ctrl+Enter 经 Queue 机制把结果追加回主任务，会话切换恢复时以快照加 registry 增长做防回退 floor。

## 修复及边界

- 本地会话的 harness runtime 行原先在首个回合才懒持久化，导致新会话的第一条输入就是 `@capability 问题` 时，子任务注册找不到主任务 runtime 行（"parent session runtime missing"）。最初版本在 TUI 引导期前置持久化；后按「仅打开 TUI 不得留下空 task 记录」的要求改为派发时懒创建——`dispatch` 读父 runtime 前经 `ensure_parent_runtime`（`crates/tui/src/remote/subtask_parent.rs`）补建：行缺失时 `INSERT OR IGNORE` 幂等建行并写入该 agent 的默认 runtime，已存在则原样返回不覆盖；首条普通提示词仍由 session 层 `persist()` 懒建行（含标题），仅打开 TUI 对 store 零写入。细节见 [2026-10-29 记录](../2026-10-29/tui-lazy-session-record.md)。
- watcher 回写只认 registry 内已登记的执行：未知 id、重复终态投递、runtime 行缺失均返回无变化，不写库；跨会话（dormant）投递只刷新快照计数，token 增量交给切换恢复 floor 吸收，避免双计。
- 摘要展示统一按字符边界截断到上限，多字节文本保持可渲染。

## 变更清单（file:line）

- 输入解析与派发：`crates/tui/src/remote/subtasks.rs:48`（`parse_mention`：仅命中已缓存 capability id，`@file` 不误触）、`crates/tui/src/remote/subtasks.rs:189`（`dispatch`：落 bookmark 会话 + 注册主任务 runtime registry）
- 注册表与事件归并：`crates/tui/src/subtask_registry.rs:71`（`apply_update` 纯函数，幂等归并）、`crates/tui/src/subtask_events.rs:25`（`SubtaskUi`：watcher 生命周期与专用 `mpsc(64)` 通道）、`crates/tui/src/subtask_events.rs:163`（UI 侧 `apply_update`）
- 轮询回写：`crates/tui/src/subtask_watcher.rs:36`（`run`：终态后把摘要与 token 用量回写主任务）
- 视图与交互：`crates/tui/src/subtask_detail.rs` + `subtask_detail_render.rs` + `subtask_detail_tests.rs`（`/subtasks` 全屏视图）、`crates/tui/src/app_popups.rs:41`（视图键路并回弹窗链）、`crates/tui/src/app_subtask.rs:88`（`append_and_pick`）/`:127`（`tick`）
- 回写主任务与徽标：`crates/tui/src/key_handler.rs:294`（Ctrl+Enter 经 Queue 追加回主任务）、`crates/tui/src/theme.rs:209-212`（`[sub x/x]` 徽标按宽度最先丢弃、不复活）
- 恢复防回退：`crates/tui/src/session_ui.rs:103`（`subtask_restore_floor`）、`crates/tui/src/app_task.rs:207-260`（切换快照 + floor 吸收 token 增量）
- 父会话行懒创建：`crates/tui/src/remote/subtask_parent.rs`（`ensure_parent_runtime`：派发作为首条输入时补建父会话行与默认 runtime；引导期不再预写入）
- 事件通道与数据模型：`crates/tui/src/worker.rs:65`（`UiEvent::SubtaskUpdate`）、`crates/core/src/harness/mod.rs:16`（`SubtaskRef` + `HarnessRuntime.subtasks`，serde default 兼容旧持久化行）

## 功能与测试对应

| 功能 | 测试 |
| --- | --- |
| `@<capability> <问题>` 解析（大小写敏感、@file 不误触、问题非空） | `parse_mention_matches_known_capability_and_question` |
| 主任务 runtime 行在首回合前就位（`@` 首条输入不再依赖先前提示词） | `fresh_local_session_has_a_runtime_row_before_its_first_turn`、`remote_bookmark_sessions_are_skipped` |
| 一次性派发建立本地 bookmark 会话并注册进主任务 registry（同 id 重复派发覆盖原位） | `register_appends_and_dedupes_by_execution_id` |
| 子执行用量聚合：优先分页消息持久化 usage，无消息类型回退事件流 `llm_usage` 帧 | `sum_message_usage_folds_all_messages`、`sum_event_usage_counts_llm_usage_frames_only` |
| 后台 watcher 轮询到终态一次性上报（服务端状态串为准，摘要/用量失败降级不失败轮次） | `terminal_subtask_settles_once_with_server_status` |
| watcher 全链路：终态 + `/result` 摘要 + 直连 `/messages` 用量分页 → 回写父 registry 与在线对话计数 | `watcher_completion_folds_tokens_into_parent_registry_and_chat` |
| 连续失败轮次上限后其余目标一次性落 error 并退出；空目标立即退出 | `settles_all_targets_as_error_after_five_failed_rounds`、`exits_immediately_without_targets` |
| 终态投递折入在线会话：计数重算、增量折入 `[tok cost]` | `terminal_update_on_current_session_folds_delta_and_counters` |
| 重复终态投递幂等（无变化、不双计） | `repeated_delivery_is_a_noop`、`apply_update_is_idempotent_for_repeated_terminal_delivery` |
| dormant 会话投递只刷快照计数；未知 id / 缺 runtime 为 no-op | `dormant_session_updates_snapshot_counters_only`、`missing_runtime_is_a_noop` |
| `[sub x/x]` 徽标：无注册不渲染；窄屏先丢 sub 后丢 call cost，`[tok cost]` 最后保留 | `subtask_badge_hidden_when_total_zero`、`subtask_badge_shows_completed_ratio_after_call_cost`、`subtask_badge_drops_first_but_call_cost_remains`、`subtask_tight_width_still_keeps_tok_when_call_cost_drops` |
| 切换恢复防回退 floor：快照总额 + dormancy 期间 registry 增长，不回退也不双计 | `switch_away_and_back_never_regresses_below_snapshot_plus_registry_growth`、`subtask_restore_floor_adds_only_dormant_growth`、`snapshot_captures_all_fields` |
| 无快照的首次访问：floor 即 Σregistry | `fresh_first_visit_floor_is_the_whole_registry_sum` |
| `/subtasks` 全屏视图：列表选择/滚动边界、Esc 关闭、键位按类型路由 | `selection_clamps_and_resets_scroll`、`page_scroll_is_bounded_by_last_detail_lines`、`esc_closes_and_enter_routes_by_kind`、`key_is_inert_while_the_view_is_closed` |
| `r` 刷新快照合并；时间线按序有界带省略标记 | `apply_update_merges_entry_and_snapshot`、`timeline_rows_maps_frame_shape_in_order`、`timeline_rows_keeps_last_limit_with_marker` |
| `c` 取消（终态前允许，向服务端发 interrupt）；Agent/Operator 才可进入对话 | `is_conversational_covers_agent_and_operator_only` |
| 视图渲染：空 registry 占位不 panic、分区标题与内容、按显示宽度换行 | `empty_registry_renders_placeholder_without_panic`、`detail_sections_render_headers_and_content`、`wrap_lines_breaks_on_display_width_not_chars` |
| Ctrl+Enter 追加返回：终态条目经 Queue 机制投递主任务（运行中排队不打断），非终态只提示 | `ctrl_enter_appends_selected_and_only_with_entries`、`terminal_entry_from_same_session_appends_to_queue_items`、`non_terminal_entry_flashes_and_does_not_append`、`terminal_entry_from_child_session_returns_parent_pick` |
| 从子会话 Enter 进入对话并携带主任务返回指针 | `enter_descends_into_the_selected_child_session` |
| 追加行文本格式（换行折叠、未完成占位、超限截断） | `append_text_renders_exact_format_and_collapses_lines` |

## 验证范围

定向检查覆盖本次改动及直接影响链路；提交前另按 submit 前置 gate 跑全 workspace 构建/静态检查/测试。

| 实际命令 / 目标 | 结果 |
| --- | --- |
| `cargo test -p opencoder-tui --lib subtask` | 43 项通过（registry/events/watcher/detail/app_subtask/徽标/watcher 集成） |
| `cargo test -p opencoder-tui --lib app_bootstrap` | 2 项通过（原前置持久化 2 项随懒创建方案移除，见 2026-10-29 记录） |
| `cargo test -p opencoder-tui --lib remote::tests::subtasks` | 2 项通过 |
| `cargo test -p opencoder-tui --lib remote::subtasks` | 5 项通过（含懒创建 2 项：首输入建行 / 已存在不覆盖） |
| `cargo test -p opencoder-tui --lib session_ui` | 36 项通过 |
| `cargo test -p opencoder-tui --lib theme` | 21 项通过 |
| `cargo test -p opencoder-tui --lib render::tests::tok_cost` | 12 项通过 |
| `cargo test -p opencoder-tui --lib command` | 70 项通过 |
| `cargo test -p opencoder-tui --test session_switch_restore --test subtask_restore_floor` | 3 + 2 项通过 |
| `cargo test -p opencoder-core --lib harness` | 10 项通过 |
| `cargo clippy -p opencoder-tui -p opencoder-core --all-targets -- -D warnings` | 通过，零警告 |

### 提交前全量 gate（submit 契约）

| 命令 | 结果 |
| --- | --- |
| `cargo build --workspace` | 通过 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过，零警告 |
| `cargo test --workspace` | 见下方记录 |

## Impact Surface

- `crates/tui`（输入解析、事件循环、渲染、会话切换、命令、主题）与 `crates/core` 的 `harness` 数据模型；对不使用 `@capability` 语法或 `/subtasks` 的既有路径为零行为变更（`@file` 提及、无 subtask 时的状态栏渲染均有回归测试锚定）。
- 持久化兼容：`HarnessRuntime.subtasks` 带 serde default，旧 runtime 行反序列化不受影响；回写仅追加 registry 内已登记执行，不触碰他人 runtime 行。
- 新增 dev 依赖：`crates/tui/Cargo.toml` tokio `test-util`（仅测试，paused-clock 用例）。

## Related Docs

- `rules/README.md`、`rules/04-dag-execution-contract.md`（未改动 DAG 语义，仅 harness 数据模型扩展）
- 同批迭代记录：`features/changelog/2026-10-28/spa-trustworthy-list-loading.md`
