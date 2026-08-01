# fix(session): 进程内续跑不再遗留悬空 tool_calls（provider HTTP 400）

## 背景

OpenAI 兼容 chat API 要求每条 assistant `tool_use` id 必须被匹配的
`tool_result` 应答；transcript 中出现未应答的 `tool_calls[].id`（如硬中断
发生在一轮工具批执行中途）时，下一轮 LLM 请求会直接 HTTP 400。

悬空 id 的两条产生路径与现状：

1. **新进程恢复**（`resume::resume`，`session resume` / `--continue`）：
   已有悬空合成逻辑，会在重建 session 时为未应答 id 合成 error
   `ToolResult`，路径闭环。
2. **进程内续跑**（web drain / TUI 双击 Esc 后继续 / CLI 重试）：drain
   turn 走 `run_with_registry` → `replay_cancelled_tasks`，它只回填
   `Cancelled` subagent task 的 `task` id；硬取消分支里被丢弃的**非 task**
   工具结果（bash/web 等正在执行的工具被 mid-tool hard cancel 打断）无人
   回填，id 永久悬空 → 下轮请求 HTTP 400。

## 变更

### `crates/session/src/dangling_tools.rs` — 新增 Fix B 模块（128 行）
悬空 `tool_use` 对账逻辑的唯一归属地，纯函数 + 幂等入口：

- `replayable_task_ids_from_records(&[SubagentTaskRecord]) -> HashSet<String>`：
  过滤出仍可 replay 的 subagent task id（`Running` / `Cancelled`）。其
  `task` id 悬空是**故意**的——下一轮由 `replay_cancelled_tasks` /
  `resume_and_replay` 回填；若合成 error 会永久应答 replay 依赖保持开放的 id。
- `dangling_tool_use_results(messages, &replayable) -> Vec<ContentBlock>`：
  纯函数，按 transcript 顺序为每个无匹配 `tool_result` 且不在 replayable
  集的 `tool_use` id 生成合成 error `ToolResult`（`DANGLING_RESULT_MSG =
  "session interrupted: tool result missing"`，与 resume 侧文案一致）。
- `reconcile_dangling_tool_uses(session)`：幂等入口——持久化合成结果
  （`synthetic=true`），Store-less 会话时 replayable 集为空 → 每个 id 都被
  应答（含 task）。

### `crates/session/src/runner/mod.rs` — Fix A 硬取消分支源头修复（641 行）
hard-cancel 分支（中断进行中的工具批）现在先用
`replayable_task_ids_from_records(store.list_subagent_tasks(...))` 过滤
`tool_blocks`，再把**不可 replay** 的工具结果作为一条 Tool message 落盘，
最后才发 `Status("interrupted")`。只有 replayable 的 `task` id 保持悬空
（留待下轮 replay/abandon）。

### `crates/session/src/runner/mod.rs` — 进程内安全网接线
`run_with_registry` 在 `replay_cancelled_tasks` 之后、新 user 输入落盘之前
调用 `reconcile_dangling_tool_uses(session)`：即使硬取消分支漏过任何 id，
续跑 turn 历史也保持 well-formed，下轮 LLM 请求不再 400。

### `crates/session/src/resume.rs` — 复用纯函数（717 行，净 −34）
裸 `resume` 的悬空计算替换为 `dangling_tools` 纯函数调用，持久化行为不变。

## 测试覆盖

| 文件 | 测试名 | 断言 |
|------|--------|------|
| `tests/hard_abort_mixed_batch.rs` | `hard_cancel_mixed_batch_records_non_task_result_then_continue_is_wellformed` | run 1 硬取消混合批：非 task 的 `call_2` 结果被记录、仅 `task-1` 悬空、DB 任务 `Cancelled`；run 2 续跑后无悬空 id，`assert_requests_well_formed` 校验每个请求无未应答 `tool_calls[].id` |
| `tests/hard_abort_mixed_batch.rs` | `in_process_continue_reconciles_preexisting_dangling_non_task` | 手工构造孤儿 `tool_use`，续跑 turn 在 transcript index 2 处出现合成 error ToolResult，无悬空 |
| `tests/hard_abort.rs` | `cancel_hard_aborts_a_running_tool`（扩展） | Store-less 无悬空 + `call_1` bash 结果被记录（Fix A 在无 store 场景同样生效） |
| `scripts/e2e/web_scenarios.py` | E15 强化 | `_assert_tool_pairs`：HARD 校验每个 `tool_use` id 都被应答（`exclude_task` 控制是否豁免 replayable task）；E15 在首个 `tool_use` 后 POST /interrupt 再续跑，最终 transcript 全量无悬空 |

## 回归结果（rules/02-regression-gate）

- `cargo test --workspace` → **1519 passed / 0 failed**
- `cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- `cargo build --workspace` → 干净

> 注：working tree 存在无关的后台改动（TUI/store 等，`Commit:
> (working-tree, pre-initial-commit)` 约定）；`web_extract.rs` 为既存
> untracked 文件，本次仅机械修复其 clippy lint 以解锁 workspace 门禁。
> E2E `scripts/e2e-glm.sh --only web` 需 `ZHIPU_API_KEY`（手动/CI 执行）。
