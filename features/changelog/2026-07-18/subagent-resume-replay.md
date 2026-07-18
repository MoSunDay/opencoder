# Subagent 续跑：resume 时重放中断的子任务

## 背景

父会话在 subagent（`task` 工具）运行途中被硬中断（崩溃 / Ctrl+C / 双击 Esc 后退出）时，`subagent_tasks` 行停留在 `Running`，父 transcript 留下一条无人应答的 `task` `tool_use`。旧 `resume()` 仅把这些卡住的 task 标为 `Failed("(interrupted)")`、子任务的工作成果被丢弃，父 transcript 靠悬空 reconcile 合成一条 error `tool_result` 了事——续跑时模型看到的是「子任务失败」，而非子任务的真实输出。

需求：**resume 时真正把中断的子 agent 续跑完，把结果回填进父 transcript**，使父会话能像子任务从未中断一样继续。

## 变更

### 新增 `resume_and_replay`（`crates/session/src/resume.rs`）

高层 resume 入口，签名与 `resume()` 一致（`store, id, config, client, working_dir -> Result<SessionState>`），TUI / CLI / web 四个 resume 调用点全部改走它：

1. `list_subagent_tasks(id)` 取父 session 所有 `Running` 子任务（按 `seq` 升序 = 派发顺序）。
2. 对每个 Running 子任务：`resume(child)`（从子 session 持久化 transcript 重建）+ `run(child, "")`（**空 prompt = 续跑**，不注入新 user 消息，子从断点继续）。
3. 把所有续跑结果**批进单条 Tool message**回填进父 session（mirrors `run_loop` 把一轮工具结果收进一条 `tool_msg` 的语义，结果按 `seq` 顺序确定），并 `complete_subagent_task`。
4. 最后调用低层 `resume()` 重建父 `SessionState`。

**关键不变式**：回填发生在裸 `resume()` 之前，故 `resume()` 跑到时已无 `Running` task、且每个 task `tool_use` 都已 answer——`resume()` 的「卡住 task 标 Failed」与「悬空 `tool_use` 合成 error」两条 reconcile 路径对该 turn 失活，不会与回填打架。

### 不需要递归 / 深度守卫

子 agent（explore / build）的工具集**不含 `task`**（`crates/core/src/agent.rs`），无法派发孙任务，最大就一层。续跑逻辑用同一 `resume_and_replay`（内部对 child 调 `resume`）即可自然终止——**无需显式深度上限、无嵌套测试用例**。原方案的「深度上限 8」「`replay_recurses_into_nested_subagent`」属于过度设计，已剔除。

### 低层 `resume()` 保持不变

`resume()` 作为安全网保留（卡住 task 标 Failed + 悬空 reconcile），其既有测试 `resume_marks_stuck_running_subagent_as_failed` 仍直调 `resume()` 验证此回退行为。直接被外部调用的入口都已切到 `resume_and_replay`。

### 子续跑事件落库

`replay_child` 复用 `run_subagent` 的 pattern：子续跑产生的事件 buffer 后按发射顺序 `append_event` 到子 session，保证子 transcript + 事件流可完整 replay。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Running 子任务被续跑 + 结果回填 + 状态转 Completed | `resume_and_replay_continues_running_child_and_backfills_result` | `session/tests/resume_replay.rs` |
| 回填后父 transcript 无悬空 tool_use、子 transcript 增长 | （同上，多断言） | `session/tests/resume_replay.rs` |
| 已 Completed 子任务不被重跑 | `resume_and_replay_leaves_completed_tasks_untouched` | `session/tests/resume_replay.rs` |
| 无 Running task 时等价于普通 resume | `resume_and_replay_no_running_tasks_just_resumes` | `session/tests/resume_replay.rs` |
| 多个 Running 子任务续跑 + 结果按 seq 顺序批进单条 Tool message | `resume_and_replay_replays_multiple_children_into_one_backfill_message` | `session/tests/resume_replay.rs` |
| 低层 `resume` 仍把卡住 task 标 Failed（回退安全网，回归保护） | `resume_marks_stuck_running_subagent_as_failed` | `session/tests/resume_reconcile.rs` |

## Gate

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 580 passed / 0 failed（基线 575 + 新增 4 个 resume_replay 测试；+1 为工作树既有异步测试的计数抖动） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace` | 零错误 |
| 行数 gate | `resume.rs` 357 行（≤800）；新文件 `resume_replay.rs` 383 行（≤400） |
