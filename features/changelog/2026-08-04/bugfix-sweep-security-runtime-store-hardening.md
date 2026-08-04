# bugfix sweep: 10 安全 / 运行时 / Web-Store / 健壮性缺陷修复

## 背景

本轮对全 workspace 做了一次深度缺陷扫描，确认并修复 10 个潜伏缺陷
（4 批次：P0 安全 / P1 运行时 / P2 Web-Store / P3 清理）。所有修复均遵循
纯函数式原则、附回归测试。测试拆分至独立文件以满足行数 gate。

修复后：1801 tests / 0 failed / 1 ignored / 0 clippy warnings。

## 变更

### Batch 1 (P0 Security) — bash_guard 绕过

**文件**: `crates/session/src/bash_guard.rs`

plan 模式依赖 bash_guard 拦截所有写操作，但以下类别可绕过分类器：

1. **`install`/`truncate` 未列入 MUTATING_COMMANDS** — `truncate file` 直接清空文件，
   `install` 可覆盖文件，均未被拦截。
2. **shell 解释器 `-c`/`-s`** — `bash -c 'rm file'` 逃逸 plan 模式限制。
3. **脚本解释器 `-c`/`-e`/`-E`** — `python3 -c '...'`、`node -e '...'`、`perl -e '...'`
   可执行任意代码；含组合 flag 如 `perl -pe`。
4. **`xargs`** — `echo file | xargs rm` 无条件拦截（xargs 总是执行子命令，无只读路径）。
5. **`find -exec`/`-execdir`/`-delete`/`-ok`/`-okdir`** — `find . -delete` 可递归删除。

### Batch 2 (P1 Runtime) — compaction / 算术下溢

**文件**: `crates/session/src/runner/mod.rs`, `compaction.rs`, `lib.rs`, `plan_handoff.rs`

6. **compaction 失败后 fall-through 到 LLM 调用** — 原实现在 `compact()`
   返回 `Err` 时仅 emit Error 事件后继续 `run_one_llm_call`，导致超限 transcript
   必然触发 context-length 400。改为 3 次重试（`for attempt in 0..=2`），
   最终失败返回 `Err`，不再 fall-through。
7. **compaction metadata 持久化错误被吞** — `let _ = store.update_session(...).await`
   静默丢弃 DB 错误。改为 `.context("persist compaction metadata")?` 向上传播。
   同时修正 `after_compaction` 调用顺序（先更新内存状态再写 DB，保证持久化失败时
   内存状态仍一致）。
8. **`store_message_count` 算术下溢** — `skip + len - 1` 在 `len == 0` 时下溢 panic。
   改为 `len.saturating_sub(1)`。`plan_handoff.rs` 中同一模式同步修正。

### Batch 3 (P2 Web/Store) — DB 错误误分类 + 跨 session 查询

**文件**: `crates/web/src/api.rs`, `crates/store/src/libsql_store/inputs.rs`

9. **3 处 `.ok().flatten()` 将 DB Err 误判为 None** — `messages_response`、
   `ensure_session_row`、`get_events` 中 `store.get_session().await.ok().flatten()`
   把数据库错误（连接断开、磁盘满等）静默当作「session 不存在」处理，返回 404 而非 500。
   改为显式 `match`：`Err` → 500 + 错误描述，`None` → 404。
10. **`last_input_seq_in_tx` 查询未限定 session** — 原 SQL `SELECT MAX(seq) FROM
    session_inputs` 无 `WHERE session_id` 跨 session 返回全局最大 seq，导致新 session
    的首条 input 可能复用上一个 session 的 seq 值。加 `WHERE session_id = ?` 限定。

### Batch 4 (P3 Cleanup) — PID guard / 重复 Error / 死代码

**文件**: `crates/session/src/tools/bash.rs`, `crates/llm/src/client.rs`, `lib.rs`

11. **`child.id().unwrap_or(0)`** — PID 为 None 时回退到 0（init 进程），
    `kill_all` 可能误杀。改为 `match child.id()` 返回 `ToolOutput::err`。
12. **LLM stream 终止路径重复 `LlmEvent::Error`** — `run_stream` 的 `Err(Connect(e))`
    分支先 `tx.send(Error)` 再 `return Err(e)`，而 `chat_stream` 的 spawn 任务也捕获
    返回的 `Err` 并 emit Error，导致客户端收到两次 Error 事件。移除 `run_stream` 内的
    重复发送。同时清理 dead code：移除未使用的 `ChatParams` struct（`client.rs`）
    及其 re-export（`lib.rs`）、移除 `flushed_any` dead variable。

## 测试覆盖

| # | Bug | 测试名 | 文件 |
|---|-----|--------|------|
| 1 | install/truncate | `install_and_truncate_blocked` | `crates/session/src/bash_guard_security_tests.rs` |
| 2 | shell -c/-s | `shell_interpreters_with_c_flag_blocked` | `crates/session/src/bash_guard_security_tests.rs` |
| 2 | shell -c/-s | `shell_interpreters_with_s_flag_blocked` | `crates/session/src/bash_guard_security_tests.rs` |
| 3 | script interpreters | `script_interpreters_with_exec_flag_blocked` | `crates/session/src/bash_guard_security_tests.rs` |
| 4 | xargs | `xargs_always_blocked` | `crates/session/src/bash_guard_security_tests.rs` |
| 5 | find -exec/-delete | `find_with_exec_or_delete_blocked` | `crates/session/src/bash_guard_security_tests.rs` |
| 1-5 | compound bypass | `interpreter_in_compound_command_blocked` | `crates/session/src/bash_guard_security_tests.rs` |
| 8 | store_message_count | `store_message_count_no_synthetic_head` | `crates/session/src/lib.rs` |
| 8 | store_message_count | `store_message_count_with_summary_seq` | `crates/session/src/lib.rs` |
| 8 | store_message_count | `store_message_count_with_handoff_seq` | `crates/session/src/lib.rs` |
| 8 | underflow guard | `store_message_count_empty_with_summary_seq_does_not_overflow` | `crates/session/src/lib.rs` |
| 8 | underflow guard | `store_message_count_empty_with_handoff_seq_does_not_overflow` | `crates/session/src/lib.rs` |
| 7 | compaction metadata Err propagation | `compact_returns_err_when_store_rejects_metadata_persistence` | `crates/session/tests/compaction_error_propagation.rs` |
| — | admitted_seq per-session scoping | `admitted_seq_is_scoped_per_session_while_global_seq_is_monotonic` | `crates/store/tests/inputs_integration.rs` |
| — | web compact persists summary | `compact_returns_ok_and_persists_summary` | `crates/web/tests/web_api_ops.rs` |
| — | web handoff persists boundary | `handoff_persists_boundary_when_plan_exists` | `crates/web/tests/web_api_ops.rs` |

**当次实跑**: `cargo test --workspace` → 1801 passed; 0 failed; 1 ignored。
