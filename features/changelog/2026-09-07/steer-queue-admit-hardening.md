# steer/queue 提交链路加固：跨进程 admit 竞态修复 + TUI 离环 steer 与失败可见性

## 问题与行为

### 根因（store 层）：`inputs.rs` deferred BEGIN 读改写升级竞态

`Store::admit_input` → `inputs.rs::admit` 的事务用 deferred `BEGIN`：先 `next_admitted_seq` SELECT 建立读快照，再 INSERT 升级写锁。同一 db 文件只要存在第二个连接（同 workdir 第二个 TUI 实例、`opencode run/session` CLI 进程、web daemon），在 SELECT→INSERT 窗口内提交，写锁升级即返回 **SQLITE_BUSY(_SNAPSHOT)——`busy_timeout=30000` 对快照失效不重试**，`admit_input` 直接 Err（TUI 表现：概率性 `⚠ steer submit failed` / queue 提交失败 flash）。`promote`/`unpromote`/`mark_recorded`/`promote_next_queued`/`swap_input_order` 同病（会让 `claim_steers` 偶发 promote 失败 → steer 提交成功却"没生效"）。同文件 `admit_once`/`claim_next_queue` 早已是 `BEGIN IMMEDIATE`（注释明说 serializes independent store instances）——本次把 `inputs.rs` 全部写事务对齐为 `BEGIN IMMEDIATE`：跨进程竞争由 busy_timeout 化为等待，同时消除跨进程 `admitted_seq` 读改写竞态。零行为面变化（除错误消失）。

### TUI 三个放大缺陷

- **键盘 steer 内联等待**：父会话 Enter-while-running steer 原在 select 循环内联 `await admit_input`（db_lock 争用时冻结渲染/输入），与 `queue_admitter` 模块文档自述的反模式相悖。新增 `steer_admit.rs`：复用同一 admitter actor，乐观行进 `chat.steer_items`（负 temp seq）+ completion 对账（`reconcile_ok`/`reconcile_err` 共用），失败 flash + 图片回滚，并顺带获得 `idle_rekick` 的 stranded 行重启（此前仅 queue 有）。`SteerConsumed` 事件现在也喂 `note_consumed` 台账（消费竞态下 completion 丢弃而非复活临时行）。结构上仍不可能触发 turn interrupt（`>` 按钮路径 `fire_steer_interrupt` 不变）；subagent steer 因 gate reserve/commit 语义保持内联。
- **queue try_send 失败静默**：channel 满/actor 消失时临时行+图片被回滚但无任何提示，composer 已清空。`handle_queue` 现返回 `bool`，全部 4 个调用点（`app.rs` Queue 分支、`app_submit.rs`、`app_loop_actions.rs` ×2）flash `⚠ queue submit failed — recover text with ↑ history`（图片已回滚，文字可 ↑ 恢复）。
- **会话切换幽灵行**：在途 `AdmitDone` 会把旧会话的行 `Reinserted` 进新会话面板。`AdmitReq`/`AdmitDone` 增加 `session_id`，`apply_done` 对账时 session 不匹配直接丢弃（旧行留在旧会话 pending，切回可见；stash 的图片丢弃而不注入新会话 composer）。

## 测试覆盖

| 功能 | 测试 | 文件 |
| --- | --- | --- |
| 双 `LibsqlStore` 实例同库文件并发 admit + 交叉写提交零 Err、`admitted_seq` 恰为 1..=N 无重复乱序（multi_thread runtime；deferred BEGIN 下 3/3 稳定复现旧失败，修复后 10+ 连跑全绿） | `cross_instance_admits_never_busy_error` | `crates/store/tests/inputs_cross_instance_serialized.rs` |
| 跨实例并发 admit 后交替 `claim_next_queue` 零错误、50 个 seq 全唯一 | `cross_instance_claim_after_concurrent_admits` | 同上 |
| steer 离环提交：临时行 + 图片入 stash / 死通道回滚（行删除、图片恢复） | `submit_ok_appends_temp_row_and_stashes_images`、`submit_on_dead_channel_rolls_back` | `crates/tui/src/steer_admit.rs` |
| steer 对账：Ok Replaced（queue 镜像不受扰）/ Err 删行 + 图片恢复 + `STEER_SUBMIT_FAILED_FLASH` / 消费竞态 DroppedConsumed 不留幽灵 | `apply_done_steer_ok_replaces_temp_row`、`apply_done_steer_err_restores_and_flashes`、`apply_done_steer_consumed_race_drops_temp_row` | 同上 |
| 会话切换竞态：mismatch Done 双镜像不动、无 flash、inflight 清空且图片不注入新 composer；queue 回归守卫 | `apply_done_session_mismatch_drops_without_restore`、`apply_done_queue_done_reconciles_queue_mirror` | 同上 |
| actor store 失败路径：删行 + flash + 图片恢复 | `actor_failure_path_flashes_and_removes_row` | `crates/tui/src/queue_admitter_fail_tests.rs` |
| `handle_queue` 失败返回值与 flash、`apply_done` session 标签/steer 镜像分派 | `queue_admitter` 既有测试更新 | `crates/tui/src/queue_admitter.rs` |

## 验收结果

- `cargo test -p opencoder-store`：43 个测试二进制全绿（含新集成测试 2 例）；`cargo clippy -p opencoder-store --all-targets` 无新告警。
- `cargo test -p opencoder-session`：101 个测试二进制全绿（steer_followup / input_delivery_recovery / steer_reabsorb / bare_steer_short_circuit / steer_batch_recovery / parent_steer_terminal / parent_turn_cancel_steer / subagent_steer / steer_skill_deferral 全部通过）。
- `cargo test -p opencoder-tui`（含 `--tests`）：1788 passed / 0 failed（1698 unit + 90 integration，含 `tests/queue_admit_offloop.rs` 原样通过）；`cargo build -p opencoder-tui` 零告警，`cargo fmt --check` 干净。
- 工作区 `cargo build --workspace` 在 `opencoder-dag-runtime` 失败（`StepKind::Python`/`rustpython_vm`）：为他人进行中的 python-step 工作树改动，与本轮无关（本轮未触碰 dag/dag-runtime/project/web）。
- 行数预算：`app.rs` 800（恰在迭代上限内）、`queue_admitter.rs` 691、新文件 `steer_admit.rs` 265 / `queue_admitter_fail_tests.rs` 149 / `inputs_cross_instance_serialized.rs` 192。
