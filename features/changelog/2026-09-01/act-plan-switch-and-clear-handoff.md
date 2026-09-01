Commit: 8709349 (working-tree, Ctrl+T 模式切换与 plan→act 清上下文交接)

# act/plan 状态切换与执行交接修复

## 背景

plan 模式恢复后，TUI 缺少保留上下文的直接 act/plan 切换键；同时
`/act_clear_context` 与 Shift+Tab 只折叠 transcript、不切到 act，导致用户明确
要求“保留计划并执行”后仍停留在只读 plan。steer 入口还会把带保留内容的
ClearContext 当成纯控制命令，三种 ingress 语义不一致。

## 变更摘要

- 恢复可重绑定的 `keymap.switch_mode`，默认 Ctrl+T。act→plan、plan→act 都保留
  transcript 与 composer draft，并复用 `/act`、`/plan` 的 busy gate、持久化与
  状态反馈；缺字段旧配置自动补默认，退役的两个 tab 变体继续忽略。
- `/act_clear_context`（legacy `/clear_context`）在 plan 下调用
  `handoff::reset_to_directive`：只保留最新真实计划，切换并持久化 act，依次发
  `TranscriptReset`、`AgentSwitch("act")`，随后恰好执行一个 act LLM turn。
- Shift+Tab 继续走 5 秒可回撤确认，但落地后与上述 canonical 命令完全同路；
  composer 草稿作为复合尾部一并交接。
- act 下 ClearContext 维持中性 continuity seed、agent 不变；完全没有 assistant
  内容时保留空白哨兵且不发起 LLM。direct、queue、steer、compound 与 resume
  现在共享同一边界语义。

## 兼容性与门禁收口

- 无 schema、环境变量或数据迁移；`handoff_plan` 仍存纯计划显示文本，resume
  重建时再补执行指令前缀。
- 清上下文仍清理 active skill；steer 续跑不会把 skill tail 带入下一请求。
- 全量门禁暴露的既有环境问题同步收口：本地 HTTP process/e2e 测试显式绕过
  进程代理；bash 注册表测试统一串行；copy-wrap 测试从当前 crossterm 输出派生
  等价 reset 后缀；web handler 改为直接返回 `Response`，消除大 Err clippy 告警。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Ctrl+T 双向纯切换并保留 draft | `ctrl_t_toggles_act_and_plan_without_touching_draft` | `crates/tui/src/key_handler_running_mode_tests.rs` |
| Ctrl+T 进入统一 running/subagent gate | `ctrl_t_reaches_app_gate_while_running_or_subagent_focused`、`mode_switch_while_running_is_busy_gated` | `crates/tui/src/key_handler_running_mode_tests.rs`、`crates/tui/src/app_loop_tests/switch_gate_tests.rs` |
| 默认/自定义 `switch_mode` 与旧配置补默认 | `keymap_without_switch_mode_uses_ctrl_t_default`、`from_config_respects_custom_mode_switch_spec` | `crates/core/src/config/keymap.rs`、`crates/tui/src/keymap_tests.rs` |
| plan ClearContext reset→act→单次执行 | `plan_clear_context_hands_off_and_executes_under_act` | `crates/tui/tests/act_clear_context_fold.rs` |
| idle/queue/steer/compound/resume 交接一致 | `plan_idle_bare_clear_hands_off_and_persists`、`plan_queue_drain_clears_before_real_prompt`、`plan_steer_clear_hands_off_and_executes`、`plan_compound_clear_runs_rest_under_act` | `crates/session/tests/clear_context_agent_kept.rs` |
| 无计划空白边界零 LLM | `plan_sentinel_clear_stops_without_llm` | `crates/session/tests/clear_context_agent_kept.rs` |
| 切 act 后解除 plan 写门 | `clear_switches_to_act_and_unblocks_next_write` | `crates/session/tests/clear_context_bash_gate.rs` |
| steer 清上下文不继承 skill tail | `steer_clear_context_mid_run_leaves_next_run_tail_free` | `crates/session/tests/skill_tail_cleared_after_run_end.rs` |
| 真实 daemon + HTTP + OpenAI SSE 的 plan→act 交接、重启恢复边界 | `real_server_clear_context_executes_preserved_plan_in_act` | `tests/running_mode_switch_e2e.rs` |
| 本地进程/relay 测试不受代理污染 | `smoke_script_two_process_nodes_flow_passes`、`node_messages_relay` 4 tests | `tests/nodes_smoke_proc.rs`、`crates/web/tests/node_messages_relay.rs` |

- 全量回归：`cargo test --workspace` → 3784 passed / 0 failed（245 suites）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 成功

## 本地部署

- `cargo build --release --bin opencoder` 成功，产物版本为
  `opencoder 0.1.0 (8709349-dirty)`。
- release 产物已原子替换 PATH 首选项 `/root/.local/bin/opencoder`；部署后 SHA-256
  为 `8aefc521771f8ea874557a1052653706163836dfcf964908dc31bd867543b185`，与构建产物一致。
- 原二进制保留在
  `/root/.local/bin/opencoder.backup-before-act-plan-20260901`，可用于回退；部署时已在运行的
  进程仍使用旧映像，重新启动后加载新版本。

## Related Docs

- [agents/session](../../../agents/session/index.md)
- [agents/tui](../../../agents/tui/index.md)
- [agents/core](../../../agents/core/index.md)
