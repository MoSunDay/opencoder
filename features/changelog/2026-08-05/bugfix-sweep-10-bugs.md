# 仓库静态深审 bug 扫除 — 10 项确定性/高置信缺陷修复

## 背景

标准门禁（`cargo build` / `clippy` / `test --workspace`）全绿，但静态多路审计发现一批"测试覆盖不到、运行时才暴露"的潜在缺陷。本轮逐一核实并修复 **10 个**已确认 bug（无臆测），每项配回归测试（红→绿验证）。

## 变更

### Tier 1 — 确定性 panic / 命中错位

- **A1 web_fetch truncate panic**（`session/src/tools/web_fetch.rs`）：`text.truncate(BODY_LIMIT)` 在 2 MiB 非 UTF-8 字符边界处 panic。退到最近字符边界再截断。`truncate_body` helper 及其回归测试抽至默认编译的 `session/src/tools/truncate.rs`（原置于 `#[cfg(feature = "browser")]` 的 `web_fetch.rs` 内，标准 `cargo test --workspace` 门禁无法触及）；`web_fetch` 经 `super::truncate::truncate_body` 调用。
- **A2 Image 块空 rendered 行数错位**（`tui/src/chat.rs`）：`collect_headers` 对空 rendered Image 计 2 行，实际渲染 3 行（多了 "(unable to render)" 占位行）→ 鼠标命中偏移 1。修正为计 1 内容行。
- **A3 未完成 Assistant 尾部 \n 行数高估**（`tui/src/chat.rs`）：`raw.split('\n').count()` 未弹尾部空串，而 `flatten_with` 弹掉了。抽出共享 `assistant_rows(raw)` 给两处复用，杜绝分叉。

### Tier 2 — 安全假阴性 / 功能错误

- **B1 plan 守卫 wrapper 绕过**（`session/src/bash_guard.rs`）：`classify_segment` 只剥 sudo/doas，不剥 `env`/`nohup`/`timeout`/`nice`/`command`/`strace`/`ionice` → `env rm file` 等被误判 ReadOnly，写操作绕过 plan 守卫。将 `strip_wrappers` 从 `ssh_pty.rs` 下沉为 `bash_guard` 共享 helper（pub(crate)），classify_segment 取命令名前先 strip。
- **B2 ssh marker 被注释吞**（`session/src/tools/ssh_pty.rs`）：`format!("{}; printf...", command, marker)` 在同一行；command 以 `# comment` 结尾时 printf 被注释 → marker 永不出现 → 瞬时完成命令误报 30s 超时。改为换行拼接（抽出 `wrap_command_with_marker` helper）。

### Tier 3 — 数据完整性 / 事件持久化

- **C1 bundle 导入 stale seq**（`store/src/bundle.rs`）：逐字 clone 源库 `summary_seq`/`handoff_seq`/`handoff_plan`/`summary`，但 append_messages 重新分配自增 seq → 引用错乱。导入前置 None（对齐 jsonl 导入）。
- **C2 bundle 导入无回滚**（`store/src/bundle.rs`）：中途失败留空 stub，幂等守卫使重试永久跳过。包裹导入体为失败即 `delete_session` 回滚（仅删当前会话，子会话各自回滚）。
- **C3 merge u32 截断**（`core/src/config/merge.rs`）：`max_consecutive_failures = v as u32` 缺 `.min(u32::MAX as u64)`；值 4294967296 截断为 0 = 静默禁用守卫。补钳制（3 个兄弟字段已钳制）。
- **C4 drain cmd 事件不入库**（`web/src/handle.rs`）：`apply_drain_cmd` 广播闭包只 `tx.send` 不 `sink.push` → `/compact`、`/handoff` 等事件不入 session_events 表，`?after=` 重放丢失。传入 sink 并补 `sink.push(&ev)`（对齐主 run 回调）。

### Tier 4 — 转录污染 / 事件误标

- **D1 子代理超时误报 steer**（`session/src/runner/execute.rs` + `subagent.rs`）：Timeout 触发子级硬取消 token，subagent.rs 据此误判"被父 steer 重定向"，summary 错报 `cancelled: redirected by parent steer`。新增共享 `Arc<AtomicBool>` 超时标志，subagent.rs 据此走 `cancelled: timed out` 独立 summary。
- **D2 硬取消落空 assistant**（`session/src/runner/mod.rs`）：LLM 流中硬取消返回空 turn；主循环只查 `is_turn_cancelled`（turn token），未查 `session.cancel` → 空 assistant 消息落库 + 发 Done。补 hard-cancel 守卫，与 turn-cancel 分支合并处理。

## 测试覆盖

| Bug | 测试名 | 文件 |
|-----|--------|------|
| A1 | `truncate_body_respects_utf8_char_boundary` | `session/src/tools/truncate.rs` |
| A2+A3 | `mixed_sequence_alignment` / `empty_image_followed_by_tool_alignment`（+6 参数化用例） | `tui/src/chat_tests/line_accounting.rs` |
| B1 | `wrapper_commands_dont_mask_writes` / `env_with_only_assignment_is_read_only` / `exec_eval_source_still_blocked_directly` | `session/src/bash_guard_tests.rs` |
| B2 | `marker_is_on_new_line_not_swallowed_by_comment` | `session/src/tools/ssh_pty_tests.rs` |
| C1 | `import_bundle_resets_summary_handoff_seq` | `store/src/bundle.rs` |
| C2 | `import_bundle_rolls_back_on_failure` | `store/src/bundle.rs` |
| C3 | `tool_guard_max_consecutive_failures_clamps_overflow` | `core/src/config/merge.rs` |
| C4 | `drain_cmd_events_persisted_for_sse_replay` | `web/tests/web_drain_contract.rs` |
| D1 | `subagent_timeout_reports_timeout_not_steer` | `session/tests/subagent_timeout_summary.rs` |
| D2 | `hard_cancel_midstream_no_empty_assistant` | `session/tests/hard_cancel_midstream.rs` |

> 每项测试均确认 **修复前红、修复后绿**。

## Gate

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 1860 passed / 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace` | Finished，零错误 |

行数约束：迭代文件均 ≤800 行（mod.rs 799、chat.rs 799）；新增文件均 ≤400 行。
0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace` | Finished，零错误 |

行数约束：迭代文件均 ≤800 行（mod.rs 799、chat.rs 799）；新增文件均 ≤400 行。
