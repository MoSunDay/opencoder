# fix(session/store): 任务恢复不再卡死 + `task_type` 列区分父子会话

## 背景

恢复一个含未完成子任务的会话（`--continue`/`--session`/`/task` 加载）时，有概率
**完全冻结**，用户消息永不被处理。根因链：

1. `replay_child()` 用 `run_with_registry` 重跑被中断的子任务，但**无总体超时**——
   子会话每轮 LLM 仅有 600s idle timeout，跨多轮可运行数小时（致命）。
2. `resume()` 重建的 child `cancel: None`，run loop 的中断检查永不触发（致命）。
3. `flusher.await` 无超时，DB 写入慢时永久阻塞（高）。
4. CLI `--continue`/`--session` 恢复传 `replay_cancel: None`，且 Ctrl-C token 在 resume
   **之后**才挂载 → 恢复阶段完全不可中断（高）。

另：`/task` 列表过滤仅靠 `NOT EXISTS (subagent_tasks)`，存在 `create_session` 成功但
`create_subagent_task` 失败的竞态时子会话会泄漏到列表。

## 变更

### 1. 恢复超时与可中断（核心，`crates/session/src/resume.rs`）

`replay_child()` 新增 `parent_cancel: Option<&CancellationToken>` 参数并重构：

- 为 child 安装独立 `CancellationToken`（`child.cancel = Some(token)`），修复根因 2。
- 用 `tokio::select!` 将 `run_with_registry` 与 **`config.replay_timeout()`**（默认
  300s）及父级取消令牌竞速：任一非 run 分支胜出则 `child_token.cancel()` 并 drop
  run future（硬取消进行中的 LLM/tool 调用），修复根因 1。权威文本已由
  `session.record()` 持久化，部分结果可恢复。
- `flusher.await` 加 30s 超时，修复根因 3。
- 超时/取消后 `replay_child` 返回 `(text, false)`，调用方据此将 task 标记 `Failed`
  并回填 error `tool_result`，转写保持良构。

调用方同步更新：`resume_and_replay` 传 `replay_cancel.as_ref()`；`replay_cancelled_tasks`
传 `session.cancel.as_ref()`。

### 2. 配置项 `replay_timeout_secs`（`crates/core/src/config.rs` + `config/merge.rs`）

新增 `replay_timeout_secs: Option<u64>`，默认 300s（独立于 `task_timeout_secs`
1800s——恢复不应阻塞用户 30 分钟）。`replay_timeout()` 取值器 + merge 支持。

### 3. CLI 恢复可中断（`crates/cli/src/run.rs`）

`CancellationToken` 创建时机提前到 `resume_and_replay` **之前**，作为 `replay_cancel`
传入；Ctrl-C 信号挂载到同一 token。修复根因 4。

### 4. `task_type` 列区分父子会话（`crates/store/`）

新增 `sessions.task_type TEXT NOT NULL DEFAULT 'parent'` 列 + 索引
`idx_sessions_task_type`（schema v4→v5）。迁移回填：已知的 subagent 子会话
（`subagent_tasks.child_session_id` 命中者）置 `'subagent'`，其余 `'parent'`。

- `SessionMeta.task_type: Option<String>`（None → parent）；常量
  `TASK_TYPE_PARENT` / `TASK_TYPE_SUBAGENT`。
- `runner/subagent.rs` 创建子会话时 `task_type = Some(SUBAGENT)`——**创建时即标记**，
  消除竞态窗口（不再依赖事后 `subagent_tasks` 行）。
- `list_sessions` 默认过滤改为 `task_type = 'parent'`（主标记）+ 保留 `NOT EXISTS`
  双保险。

TUI/Web 路径无需改动：`replay_child` 的超时兜底覆盖所有调用方。

## 测试清单

| 行为 | 测试 | 位置 |
|---|---|---|
| 重跑超时后 task 标记 Failed + 回填 error result，且不卡死 | `replay_child_times_out_and_marks_task_failed` | `crates/session/tests/resume_replay_timeout.rs`（integration） |
| 父级取消令牌中止重跑，task Failed，迅速返回 | `replay_child_aborts_on_parent_cancel` | `crates/session/tests/resume_replay_timeout.rs`（integration） |
| `/task` 列表按 task_type 排除 subagent 子会话 | `list_excludes_subagent_children_by_task_type` | `crates/store/tests/task_type_filter.rs`（integration） |
| include_subagents=true 时子会话出现 | `list_includes_subagents_when_requested` | `crates/store/tests/task_type_filter.rs`（integration） |
| parent 默认 'parent'、child 'subagent' 持久化往返 | `parent_session_persists_default_task_type` | `crates/store/tests/task_type_filter.rs`（integration） |
| v4→v5 迁移加列、回填 subagent、建索引、版本=5 | `schema_v4_to_v5_adds_task_type_and_backfills_subagents` | `crates/store/tests/schema_v4_migration.rs`（integration） |
| `replay_timeout` 默认 300s | `replay_timeout_defaults_to_300s` | `crates/core/src/config.rs`（unit） |
| `replay_timeout` 可配置 | `replay_timeout_is_configurable` | `crates/core/src/config.rs`（unit） |

回归：本任务触及的 `session`/`store`/`core` crate 全绿——新增 8 个测试全通过，既有测试无回归（`resume`/subagent/store schema/config 改动区域均覆盖）。
注：`cargo test --workspace --all-targets` 的总通过数因工作树同时含其他进行中改动（约 50 个范围外文件，其 bash/bg 部分当前未编译/未通过）而非确定，故此处不引用单一总数；提交时仅纳入本任务相关文件。
