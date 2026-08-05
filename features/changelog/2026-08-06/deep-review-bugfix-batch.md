Commit: (working-tree, pre-initial-commit)

# 深审 bug 扫除批次 — 6 主题

## 背景
动态/静态深审发现一批确定性或高置信缺陷，分布在 session 运行时、store 持久化、web drain 生命周期与 cli skill-token 剥离路径。多为竞态、状态泄漏或返回值陈旧，每个都附带回归测试。本批次在已全绿的 gate 上一次性提交。

## 变更

### session: resume 空 backfill 守卫
- **`crates/session/src/resume.rs`**（`replay_cancelled_tasks`）：当 replay 循环因 cancelled token 立即 break、`backfill` 为空时，新增 `if backfill.is_empty() { return; }` 提前返回，避免向 transcript 写入一条空的合成 Tool 消息（providers 会拒绝空 tool_result）。

### session: compaction 后重置守卫计数器
- **`crates/session/src/runner/mod.rs`**（`run_loop`，compaction 成功分支）：compaction 用全新 summary 替换 transcript 后，清除 `doom` / `tool_failures` / `bash_timeout_first`。此前这三个计数器在循环外声明且永不重置，预 compaction turn 的陈旧 doom 签名 / 工具失败计数 / bash 超时去重会在 compaction 后误触守卫。

### store: claim_next_queue 返回陈旧 promoted_seq
- **`crates/store/src/libsql_store/inputs.rs`**（`claim_next_queue`）：返回的 `SessionInput` 在事务内 `UPDATE` 之前物化，且 SELECT 以 `promoted_seq IS NULL` 过滤，导致返回值恒为 `None`。修复为 UPDATE 后将新 `promoted_seq` 回写到返回结构体。

### store: import_bundle 误清子会话 workdir_hash
- **`crates/store/src/bundle.rs`**（`import_bundle_inner`）：条件由 `depth > 0 || workdir_hash.is_some()` 改为仅 `workdir_hash.is_some()`。此前 `depth > 0` 分支在调用方未传 override 时也会把子会话自带的 `workdir_hash` 清成 `None`，违背「每个 session 行都携带 workdir_hash」的契约。

### web: drain 生命周期竞态（2 处）
- **`crates/web/src/handle.rs`**：
  - **`drain_to_completion`**：`DrainGuard`（清 `draining` 标志）改为在 `flusher.await` 完成后才 drop，确保 store 收齐全部 session 事件后 `draining` 才读为 false，阻止 50ms 轮询 reaper 抢跑起新 drain。
  - **`admit_and_drain`**：`fire_child_cancels` 仅在「真正启动新 drain」时触发；向已运行 drain 投递 Queue 输入（Branch B）不再硬取消该 drain 在途的 subagent 子任务。

### cli: skill-token 剥离回归守卫
- **`crates/cli/tests/skill_token_stripping.rs`**：新增回归测试，锁定「未解析 `$token` 保留为字面文本、仅剥离已解析 token」的语义（对比 `extract_skill_tokens` 会剥离全部 token 的旧行为）。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 空 backfill 不写空 Tool 消息 | replay_cancelled_tasks_no_empty_tool_message_when_backfill_empty | crates/session/tests/replay_cancelled_empty_guard.rs |
| compaction 重置工具失败计数 | compaction_resets_tool_failure_counter | crates/session/tests/compaction_resets_tool_guard.rs |
| claim_next_queue 返回 promoted_seq | claim_next_queue_returns_promoted_seq | crates/store/tests/inputs_integration.rs |
| import 保留子会话 workdir_hash | import_bundle_preserves_child_workdir_hash_when_none | crates/store/src/bundle.rs |
| queue 投递不取消运行中子任务 | queue_admit_to_running_drain_does_not_cancel_children | crates/web/tests/handle_bugfix.rs |
| drain 完成后才清 draining | drain_completion_persists_events_before_clearing_draining | crates/web/tests/handle_bugfix.rs |
| 未解析 $token 保留字面文本 | unresolved_skill_token_preserved_in_prompt | crates/cli/tests/skill_token_stripping.rs |

- 全量回归：`cargo test --workspace` → 1914 passed, 0 failed, 1 ignored
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 0 warnings
- 行数：runner/mod.rs 597 ≤ 800；resume.rs 741 ≤ 800；bundle.rs 527 ≤ 800；handle.rs 460 ≤ 800；inputs.rs < 800

## Impact Surface
- session：取消任务回放与 compaction 后的守卫行为更稳健（不影响 drain 语义与 Store 抽象边界）。
- store：`claim_next_queue` 返回值与 `import_bundle` 的 workdir_hash 语义修正（不影响 Store trait 接口）。
- web：drain 生命周期竞态消除，Queue 投递不再误杀子任务（不影响 HTTP/SSE 协议）。
- cli：skill-token 剥离语义回归守卫。

## Related Docs
- [agents/session](../../agents/session/index.md)
- [agents/store](../../agents/store/index.md)
- [agents/web](../../agents/web/index.md)
- [既有相关 changelog](../2026-08-06/drain-idle-boundary-fixes.md)
