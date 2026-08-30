Commit: (working-tree, skill run-end 清除三处修复 + [active skill] tail 收敛 fallback-only)

# 修复「每个 turn 都输出 The task-plan skill is active」：run-end skill 清除补全 + tail 降级为 fallback-only

## 背景

用户报告：激活 task-plan skill 后按 shift+tab，之后每个 turn 都复述「The task-plan skill is active.」。活库探针（会话 01M16YEEMC0CE9CSX15KP7ATXG）完全闭合根因链：

1. `clear_on_run_end` 的 store 写入用 `let _ =` 吞错：`sessions.skill` 行自 23:04 起持续 SET（task-plan 全文），期间 185 个 Done 事件零次 clear 落库；
2. 每个 turn resume 从未清的行复活 `skill_prompt`（resume.rs）；
3. `tail_reminder` 每次 LLM 调用从 `skill_prompt` 生成 `[active skill]` 段（skill_context.rs → llm_call.rs），而该段指向的正文其实已经以 `[skill loaded]` 消息在上下文里——模型每轮照念。

## 实现

- **F1 清除不再吞错**：`clear_on_run_end(session, on_event)` 新签名——内存清理后对 `SessionPatch { clear_skill: true }` 做 3 次重试（100/200ms backoff），最终失败发 `SessionEvent::Status` 可见上报，绝不静默。`run_loop_one_shot` 与 autopilot 三文件（finish/phases/review_pass ×2）调用点同步更新。
- **F2 早退分支补清**：runner/mod.rs 两个绕过 one-shot 包裹的早退分支（control apply 出错分支、裸控制命令分支）返回前补 `clear_on_run_end`——裸 `/act`/`/sandbox` 不再让 crash-resume 武装的 skill 存活。
- **F3 tail 收敛 fallback-only**：`tail_reminder` 在 transcript 已含匹配 `[skill loaded]` marker（`loaded_marker_matches` + `source_paths_from_body` 集合相等）时抑制 `[active skill]` 段；compaction 折叠 marker 后自动恢复。消除每轮同义反复——正文已在上下文里，指针只在 marker 缺席（首次注入前一轮 / compaction 后）时补位。

## 契约/语义变更（有意）

- 武装 turn 的请求改为经 `[skill loaded]` 消息携带 skill 正文；`[active skill]` 段仅在 marker 不在 transcript 时出现。
- run 结束 skill 清除失败不再无声：可见 Status 事件 + 3 次有界重试。
- 裸控制命令 run 与出错 run 同样遵守 run-end skill 清除契约。

## 测试清单（全量回归）

新增回归测试：

| 修复 | 测试 | 位置 |
|------|------|------|
| F1 | `clear_retries_through_transient_store_failure`（2 败 + 1 成重试落库、无 Status 噪音） | skill_lifecycle.rs（RetryProbeStore 双） |
| F1 | `persistent_store_failure_surfaces_status`（3 次有界重试、内存仍清、Status 可见） | skill_lifecycle.rs |
| F2 | `bare_control_cmd_clears_armed_skill_in_memory_and_store`（内存 + store 行双清、零 LLM 调用） | tests/skill_early_exit_clear.rs（新文件） |
| F3 | `tail_reminder_is_fallback_only_while_loaded_marker_present`（真实 ensure_full_body_loaded 落 marker → 抑制；messages.clear 模拟 compaction → 恢复；异路径 marker 不误伤） | skill_context.rs inline |

有意更新的契约断言（F3 下武装 turn 交付机制迁移）：autopilot_review（review turn → `[skill loaded]`）、queued_skill_drain（drained turn）、skill_context_tail（3a 更名 + turn-2 tail 仅 catalog 段 + body 断言）、skill_mid_run ×3、skill_one_shot ×2、skill_tail_cleared_after_run_end ×3（新增 `has_loaded_skill_message` 助手）、steer_skill_deferral ×1。

全量回归（专用 target dir，--no-fail-fast）：

- `cargo test -p opencoder-session`：**lib 420 passed / 0 failed；89 个集成测试二进制全部 ok / 0 failed**（含此前失败的 autopilot_review、skill_context_tail、skill_mid_run、skill_one_shot、skill_tail_cleared_after_run_end、steer_skill_deferral、queued_skill_drain；subagent_timeout_cancel 首轮 3 失败经隔离复跑确认为并行负载抖动，本轮通过）。
- `cargo test -p opencoder-tui`：**全部二进制 ok / 0 failed**。

## 上线/生效说明

- 需重启 daemon/TUI 进程使新二进制生效。重启后 F3 立即终结每轮提示（marker 在场即抑制）；存量脏 `sessions.skill` 行由 F1 在首次 run 结束时落库清除，此后 resume 不再复活。
