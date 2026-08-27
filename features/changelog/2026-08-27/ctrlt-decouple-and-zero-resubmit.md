Commit: (working-tree)

# ctrl+t 结构性解耦 + 失败零重提交（F3 语义变更）

## 背景

两个问题闭环修复：

1. **ctrl+t 与 Shift+Tab 共用 `handle_switch_agent`**，仅靠一个布尔参数（`no_handoff`）区分「纯切换 / handoff 执行」——混用结构使纯切换键与执行路径存在理论耦合（旧构建中确可触达执行）。
2. **LLM 请求失败后的自动重提交**：session F3 错误路径无条件 `reabsorb_tail`（重进 drain 消费 queue/steer 并**发起新 LLM 请求**）；web drain 的有界重启循环（`MAX_DRAIN_RESTARTS=2`）同理。失败时若恰有 pending 行，用户会观察到「失败后又冒出新 turn」。

## 语义变更声明（对 F3 的有意反转）

- **旧行为**：run Err → 错误路径 re-absorb / web 有界重启 → 失败 run 内部自动发起后续消费（新 LLM 请求）。
- **新行为（零重提交）**：run Err → **直接终止，不再重吸收、不再重启**。条目一律不丢：未消费的行保持 pending；已 claim 后消费失败的行由既有 P1-3 原地 unpromote / F2 入口恢复兜底。**下一次成功 run**（新 prompt admit / 新 drain）正常消费积压行。
- 保留不变：成功路径 P1-4 `reabsorb_tail`；TUI `Done` 后 `drain_pending` re-kick；`drain_one_queued` P1-3 失败原地 unpromote；claim 抖动（store 错 → Ok + 成功路径 re-absorb）。

## 实现

### tui（crates/tui）

- 新增 `mode_switch.rs`：`handle_pure_mode_switch`（ctrl+t / t+Tab 专用）——**结构性分离**：模块内不存在 `SwitchAndStart` / `start_turn` 调用路径，纯切换在代码表示上不可能触发执行。双向 running gate（`running || subagents_running > 0`）busy 拒绝；idle 时 `sys_tokens_for` + 乐观 `fold_agent_switch` + `pure_switch_send`（flash + try_send + 同名 dedup）。
- `pure_switch_send` 提为共享尾部：`handle_switch_agent`（Shift+Tab/Alt+Tab 专用，删除 `no_handoff` 参数）的纯切换分支同走此路径。
- `app.rs` 的 `KeyAction::SwitchAgentNoClear` 臂改调 `mode_switch::handle_pure_mode_switch`；`app.rs` 796 行 / `app_loop.rs` 785 行（迭代红线内），新文件 211 行。

### session（crates/session）

- `runner/mod.rs` 错误分支删除 `reabsorb_tail`，直接 `return Err`（附零重提交注释）。成功路径 P1-4 re-absorb 原样保留。

### web（crates/web）

- `handle.rs` 删除重启循环、`MAX_DRAIN_RESTARTS`、`should_restart_drain`、`pending_input_count`；`drain_to_completion` 变为单次 run + `process_drain_cmds`（endpoint 转发的 autopilot/annotation 命令仍在 run 结束后应用）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| ctrl+t submitted-plan idle 只发 SwitchAgent、绝无 SwitchAndStart、running 保持 false | `switch_no_clear_idle_skips_handoff` | `tui/src/app_loop_tests/switch_gate_tests.rs` |
| running 中 ctrl+t 拒绝（plan→act / act→plan 双向，状态不动） | `switch_no_clear_while_running_is_noop` / `switch_act_to_plan_no_clear_while_running_is_noop` | 同上 |
| 纯切换结构性断言（通道只收 SwitchAgent） | `pure_switch_sends_only_switch_agent_despite_submitted_plan` | `tui/src/mode_switch.rs` |
| 存活子代理（running=false）ctrl+t 仍拒 | `pure_switch_blocked_while_subagent_live_even_if_running_false` | 同上 |
| 同名 dedup / 异名必发 | `pure_switch_dedups_consecutive_same_name` | 同上 |
| LLM 失败：恰 1 次请求、队尾保持 pending、下次成功 run 消费 | `llm_failure_leaves_queue_pending_without_resubmit` | `session/tests/input_delivery_recovery.rs` |
| store 失败：失败项原地 unpromote、无孤儿 | `store_failure_leaves_queue_pending_unpromoted` | 同上 |
| steer 批：失败项只消费一次（反转旧「F3 重吸收重发」断言） | `runner_consumes_batch_steers_with_failing_store` | `session/tests/steer_batch_recovery.rs` |
| web drain 失败：恰 1 次 attempt + SSE Error 透传 + 持久化 + 尾行 pending | `drain_error_never_restarts_and_keeps_inputs_pending` | `web/tests/drain_no_restart_on_error.rs`（替代 `drain_restart_on_error.rs`） |
| 下一次 drain 消费积压行至完成 | `next_drain_consumes_stranded_pending_inputs` | 同上 |

## 回归门（rules/02）

- `cargo test --workspace`：**232 套件 / 3394 用例全绿**（exit 0）。
- `cargo fmt --all --check`、`cargo clippy -p opencoder-tui -p opencoder-web -p opencoder-session --all-targets`：干净。

## 边界与风险

- **消费方感知**：依赖「失败自动重试」的 SSE 消费方需感知此语义变更（web drain 失败后 `draining=false`，条目等待下一次 admit/drain）。
- **版本残留**：解耦对旧二进制无效，须以当前 HEAD 重建后验证 ctrl+t。
- P1-3/F2 的 no-strand 兜底语义未动：任何时刻行状态要么 consumed、要么 pending，绝无 stranded-promoted 丢失。
