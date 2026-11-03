# 压缩摘要回放可见性修复（TUI）

一句话：压缩（compaction）折叠后的会话在一切"从消息列表重建视图"的路径上
（resume 重启、`/task` 切回、远端重挂载、运行中 `TranscriptReset` 重建）都会
静默丢掉压缩内容——合成摘要消息被当作"内部指令"整体跳过，用户看不到会话
被压缩成了什么。现在回放把它渲染成与直播路径完全相同的 `📝 Compaction`
可折叠块（默认收起、点击展开），并保证随后的 `Compaction(summary)` 事件
不会重复堆块。

## 变更清单

- `crates/session/src/compaction/mod.rs` — 新增公开常量
  `SUMMARY_PREFIX`（合成摘要消息的固定头行），`compaction_message` 与
  上一轮摘要检测改用该常量；前后端以此共享同一标记，不再各自硬编码。
- `crates/tui/src/session_ui/replay.rs` — `replay_one` 对
  `synthetic && display.is_none()` 的用户消息先识别压缩摘要
  （`compaction_summary_of`：剥掉前缀取正文），命中则推送
  `ChatBlock::Compaction { collapsed: true, streaming: false }`，
  其余合成消息（plan→act 交接 / clear-context 种子）维持跳过；
  摘要不进 `first_prompt`。
- `crates/tui/src/compaction_block.rs` — `finalize_compaction` 增加
  幂等去重：`TranscriptReset` 重建可能已把同一摘要渲染为回放孪生块
  （且在途回显重推后孪生块不一定是最后一块），事件到来时同文的
  非流式 Compaction 块保持原样（披露状态、用户展开均保留），
  不再追加重复块；文本不同则维持历史行为新建块。

## 行为口径

- 直播压缩（手动 `/compact` 与自动压缩）事件序为
  `TranscriptReset → Compaction(summary)`：重建先渲染孪生块，事件幂等落位，
  最终恰好一个块，与修复前"重建后再补一个块"的可见结果一致，且
  resume 后不再消失。
- 回放孪生块默认收起（与 `finalize` 新建块口径一致）；折叠/展开、
  `collapse_all_collapsible`、鼠标命中（`compaction_headers`）全部
  沿用既有 Compaction 块管线，无新状态。
- 非合成的普通用户输入即使文本以标记开头也按真实输入渲染，
  绝不误吞。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 回放渲染摘要头块（剥前缀、收起、非 user 回显） | `replay_renders_summary_head_as_collapsed_compaction_block` | `crates/tui/src/session_ui/compaction_replay_tests.rs` |
| 摘要不污染 `first_prompt` | `replay_summary_does_not_pollute_first_prompt` | 同上 |
| 非合成标记文本仍是用户输入 | `replay_non_synthetic_marker_text_stays_user_input` | 同上 |
| 其他合成消息仍跳过 | `replay_still_skips_other_synthetic_messages` | 同上 |
| 事件与孪生块不重复（孪生非末块） | `finalize_after_replay_twin_does_not_stack_duplicate` | `crates/tui/src/chat_tests/compaction_state.rs` |
| 孪生块披露状态保留 | `finalize_after_replay_twin_keeps_user_disclosure` | 同上 |
| 异文摘要仍新建块 | `finalize_with_different_text_still_pushes` | 同上 |
| resume 全链路渲染摘要头 | `resume_replay_renders_summary_head` | `crates/tui/tests/compaction_summary_replay.rs` |
| 运行中 reset+事件单块（含在途回显） | `transcript_reset_then_compaction_event_renders_single_block` | 同上 |

- 定向回归（实际执行）：
  - `cargo test -p opencoder-tui --lib` → 1810 passed / 0 failed
  - `cargo test -p opencoder-tui --test compaction_summary_replay` → 2/2
  - `cargo test -p opencoder-tui --test tok_cost_replay --test act_clear_context_fold --test subtask_restore_floor --test resume_context_used --test session_switch_restore` → 25/25
  - `cargo test -p opencoder-session --test compaction_and_model` → 10/10；
    `--test resume_reconcile --test resume_image_survival` → 8/8
- Clippy：`cargo clippy -p opencoder-tui -p opencoder-session --all-targets` → 零警告
- 提交前 gate（全量）：
  - `cargo build --workspace` → 干净（56s）
  - `cargo clippy --workspace --all-targets -- -D warnings` → 零警告
  - `cargo test --workspace --no-fail-fast` → 510 target：6086 passed /
    235 failed（49 个失败 target）。
- 全量失败归因（均非本次改动引入，已 `git stash` 在干净 HEAD 复验
  `brain_layered_e2e` 同样失败）：
  - runc 容器禁限：`error reopening /dev/null inside container:
    read-only file system`，影响 brain_*/dag_*/agent_runc 等容器型 e2e；
  - 环境文件句柄耗尽：`Too many open files`（93 处），长跑 e2e 进程
    耗尽整机 fd 上限，波及 server/operator/workload 类套件。
  - 本次改动直接链路在全量 run 中全绿：core 327/0、llm 111/0、
    session 502/0、store 1/0、tui 1810/0（含新增 9 例）。
- 测试范围说明：定向回归 + 提交前全量 gate（因载入 submit 契约要求）；
  全量失败项为环境存量，未扩大修复范围。

## 配置与依赖

- 无新增依赖；无协议、事件序或持久化变化（`SUMMARY_PREFIX` 仅把既有
  字面量收敛为共享常量，线上格式不变）。

## 回滚步骤

- 还原上述三个源文件即回滚；测试文件随之删除，无数据迁移。

## 遗留

- SPA/Web 快照路径的压缩摘要展示为另一条链路，本次未动（如需对齐另立变更）。
