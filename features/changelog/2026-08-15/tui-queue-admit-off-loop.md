# Tab/Enter 排队提交移出事件循环（queue admitter actor）

## Context

Tab 入队只在 turn 运行时发生，而这恰是全进程 DB 写最密集的时刻：Store 是单连接 + 单把 `db_lock` 串行一切 DB 操作（turn 的消息持久化、事件 flusher 批量事务、每个 subagent 的 flusher、turn 边界的 `claim_next_queue`）。TUI 事件循环在 `tokio::select!` 按键分支里内联 `.await store.admit_input(...)`，等于让 UI 渲染/按键整体排队等锁——表现为「Tab 提交有概率顿一下」。放大器：每次提交还同步全量扫描 `~/.opencoder/skills`（逐文件 `read_to_string`）跑在 UI 线程上。

## Change Summary

- 新增 `crates/tui/src/queue_admitter.rs`（389 行）：常驻 admitter actor（`spawn_admitter(store)`，串行 FIFO 消费 `AdmitReq` → `admit_input` → 回发 `AdmitDone`），UI 侧 `submit()` 以负数 temp seq 乐观上屏并 `try_send` 派发（零锁等待，actor 不可达时完整回滚）；`reconcile_ok/err` 对账（temp 原位换真实 seq、seq 去重、`consumed` 台账防止 QueueConsumed 先到时复活已消费行）、失败回滚（删镜像行 + flash + 恢复未动用的图片快照）。
- `app.rs`（797→794 行）：四个内联 admit 点（Enter-while-running 文本/纯 skill、Tab 文本/纯 skill）全部改走 actor；`KeyAction::Queue` 整臂下沉为 `queue_admitter::handle_queue`；select 新增 `admit_done_rx.recv(), if admitter_alive` 分支做对账（closed-channel 防忙转）。
- `app_loop.rs`：`fold_ui_events` 新增 `admit` 参数，`QueueConsumed` 时 `note_consumed` 记台账（128 条环形上限）。
- `queue_panel.rs`：`plan()` 对负数 temp seq 的重排/删除直接 no-op（store 行尚不存在，窗口仅毫秒级）。
- `session_ui.rs`：`SessionUiState::snapshot` 过滤负数 temp 行，会话切换快照不携带幻影行。
- `core/skill.rs`：`discover()` 走进程级缓存（目录路径 + 每个 skill 文件的 (path, mtime) 指纹），命中时只做 `read_dir`+`stat`、零 `read_to_string`；`discover_in` 保持不缓存供测试使用。惠及 Enter/Submit/Tab 全部提交路径与 `$` 菜单。

**明确不动**：Store 的 `db_lock`/连接模型与 WAL 配置（2026-07-22 changelog 已论证串行化必要性）；`steer_fire`/`subagent_input` 的 Enter-steer 同根因路径留作后续同模式迁移。

## Validation

- `cargo test --workspace --no-fail-fast` → 全绿（见下表口径，workspace 当次 163 个测试二进制 2652 passed / 0 failed）
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- `cargo build --workspace` → Finished

### 测试覆盖

| 测试 | 断言 |
| --- | --- |
| `queue_admitter::tests::reconcile_ok_replaces_temp_row_in_place` | temp 行原位换成真实 seq（位置保持） |
| `queue_admitter::tests::reconcile_ok_drops_duplicate` | 真实 seq 已在镜像（重建竞态）→ 删 temp 不复活副本 |
| `queue_admitter::tests::reconcile_ok_drops_consumed` | consumed 先到 → temp 删除不复活 |
| `queue_admitter::tests::reconcile_ok_missing_leaves_items_unchanged` | temp 不在镜像（已重建）→ no-op |
| `queue_admitter::tests::reconcile_err_removes_temp_row` | 失败删 temp 行，返回是否在场 |
| `queue_admitter::tests::note_consumed_caps_at_128` | 台账 128 条上限，丢最旧 |
| `queue_admitter::tests::restore_images_only_into_empty_pending` | 仅 pending 为空才恢复图片快照 |
| `queue_admitter::tests::submit_round_trip_without_store` | 乐观上屏 (−1, disp)、pending 图片入 stash、AdmitReq 可收、第二次分配 −2 |
| `queue_admitter::tests::submit_rolls_back_on_dead_sender` | actor 不可达：镜像行/图片/pending 全回滚 |
| `queue_admitter::tests::actor_round_trip_admits_and_reconciles` | 真 LibsqlStore：actor 落库、completion→Replaced、pending_inputs=1 |
| `queue_admitter::tests::actor_failure_path_flashes_and_removes_row` | store 失败 → flash + temp 行删除 + 图片恢复 |
| `queue_panel::tests::negative_seq_is_noop_for_every_action` | Up/Down/Delete 对 temp seq 全 no-op |
| `session_ui` `snapshot_drops_optimistic_temp_queue_rows` | 会话快照不携带负数幻影行 |
| `skill::cache_tests::cache_serves_repeat_calls_and_invalidates_on_edit` | 重复调用命中 + mtime 变更失效 |
| `skill::cache_tests::cache_invalidates_on_file_add` | 新增文件指纹失效 |
| `skill::cache_tests::distinct_roots_do_not_collide` | 单槽缓存按 root 键控不串目录 |
| 集成 `queue_admit_offloop::blocked_admit_does_not_stall_ui_loop` | admit 阻塞 120ms 期间 tick≥5、按键照常处理，completion 后 Replaced 落库 |
| 集成 `queue_admit_offloop::consumed_before_completion_does_not_resurrect` | 端到端 consumed-late 不复活 |
| 集成 `queue_admit_offloop::second_submit_lines_up_behind_blocked_first` | 第二笔提交同 tick 上屏，两笔 FIFO 完成、seq 有序落库 |

既有 flow 级测试（`app_loop_bugfix_tests` 8 处、`app_loop_tests` 7 处）机械补 `admit` 参数后全绿，断言未改。

## Compatibility

- 失败 UX：文本已在提交时进入历史（Tab 路径本就 `push_history`），flash 提示用 ↑ 找回；仅当 `pending_images` 仍为空才恢复图片快照（避免覆盖用户随后粘贴的新图）。
- 提交中的队列行（负数 seq）毫秒级窗口内 ✕/↑↓ 点击为 no-op；对账或 Done 边界重建后恢复可操作。
- skill 发现缓存对 TUI 运行中新增/编辑/删除 skill 文件均即时失效（指纹含每个文件 mtime）。
