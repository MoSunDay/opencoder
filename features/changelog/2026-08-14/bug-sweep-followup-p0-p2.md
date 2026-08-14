Commit: (working-tree, pre-initial-commit)

# 16-bug sweep 之后残留缺陷清扫（P0 挂起/数据错乱 → P1 bash 守卫 → P2 session 运行时）

## 背景
对 16-bug sweep（`625e2ea`）之后当前 HEAD 的深度复审，确认 12 项仍真实存在的逻辑缺陷（编译与测试彼时全绿）。按危害分三批修复，每项附回归测试。并行会话在途开发 `question` 工具（resume.rs/runner/* 等持有未提交改动），本批所有修改避开其在途区域，未提交、未回退任何他人改动。

## 变更

### P0 — 挂起 / panic / 数据错乱（llm + web）
- **llm `http_date.rs`**：`days_from_civil` 改 i64 符号算法（Hinnant 原式需有符号），1970 前日期不再 `u64` 下溢（debug panic / release 天文数字回绕）；新增字段范围校验（day 1–31、hour<24、min/sec<60）；核心改为纯函数 `http_date_secs_since(s, now)`，过去日期返回 `Some(0)`。
- **llm `retry.rs` + `client.rs`**：`Retry-After` 服务端延迟封顶 120s（`RETRY_AFTER_MAX_SECS`，纯函数 `retry_delay` 下限 1s、与退避取 max）——`Retry-After: 86400` 不再挂一天。
- **web `api.rs` post_prompt**：`delivery` 拼写错误由静默 `unwrap_or(Steer)`（打断运行中 turn）改为 400；`Delivery::parse` 先 `trim()`；缺省字段维持 Steer。
- **store `types.rs`/`sessions.rs` + web `api.rs`**：`SessionPatch` 新增 `clear_model`/`clear_agent`（与字段互斥，仿 `clear_summary`）及 `rollback_model`/`rollback_agent` 构造器；`post_agent`/`post_model` 三处 TOCTOU 回滚按旧值分支——旧值为 NULL（常态）时回滚此前是空操作（`None`=不更新），409 却留下新值；现在正确清回 NULL。
- **web `handle.rs`**：drain resume 失败时仅在 `subscribers == 0`（计数器与 `tx.receiver_count()` 双保险）且 map 仍持同一 `Arc` 实例时才移除 handle——存量 SSE 订阅者不再被孤儿化；`release_events_subscriber` 改饱和递减，同 id 新实例不再计数下绕到 `usize::MAX`。

### P1 — plan 模式 bash 守卫绕过（`bash_guard.rs`）
- **控制流段首 token 绕过**：`strip_leading_control` 剥离段首 `then/do/else/{/(`/`!`、`case SUBJECT in` 头与 `word)` 模式标签——`if …; then rm x; fi`、`do rm`、`{ rm x; }`、`case x in a) rm x` 不再放行。
- **wrapper 旗标绕过**：`env -i/nice -n 5/timeout -k 1 5/ionice -c 2` 跳前导旗标及带值旗标；补 `time/stdbuf/setsid` wrapper——`env -i rm x` 等不再放行。
- **管道喂解释器 stdin**：`Segment{stdin_from_pipe}` 标记管道右段，解释器无脚本文件参数（仅旗标/`-`/空）即阻断——`curl … | sh`、`cat x.py | python -` 不再放行。
- **find/sed 漏挡**：find 写文件动作补 `-fprint/-fprint0/-fprintf/-fls`；sed 就地编辑匹配覆盖 `--in-place[=suffix]` 长形态。

### P2 — session 运行时
- **autopilot `verify.rs`**：滑窗装填后 `repair_window_pairing` 修复 tool_use/tool_result 配对（丢头部孤儿 tool_result、剥尾部未应答 tool_use）——judge 请求不再 400 → autopilot 不再误判 Aborted。配对逻辑提取为 `dangling_tools::{tool_use_ids, tool_result_ids, tool_use_ids_without_result}` 共享纯函数。
- **resume `resume_and_replay`**：跨进程回放补 `answered` 集合 + handoff/compaction 边界可见性双过滤（复用 `dangling_tools::tool_result_ids`）——超时/已回填 subagent 不再重复回填 tool_result、边界下 dispatch 不再产生孤儿 tool_result（provider 400、session 永久损坏）。被滤除的 task 行保持原状态（幂等，与 `replay_cancelled_tasks` 语义一致）。
- **control_cmd `/act_clear_context`**：plan 来源门槛 `agent.kind == Plan || plan_input_count > 0`——act 模式下任意"最后一条 assistant 文本"不再被包装成已定稿计划注入，走 fresh-start 哨兵路径。

## 测试覆盖

| 缺陷 | 测试名 | 位置 |
|------|--------|------|
| http_date 下溢/校验 | `pre_1970_date_yields_zero_without_underflow`、`year_zero_yields_zero_without_underflow`、`out_of_range_fields_are_rejected`、`absurd_years_overflow_to_none` 等 9 例 | llm/src/http_date.rs |
| Retry-After 封顶 | `retry_delay_caps_server_hint_at_max`、`retry_delay_floors_zero_and_one_to_one_second` 等 4 例 | llm/src/retry.rs |
| delivery 400 | `invalid_delivery_is_a_400`、`padded_queue_delivery_is_admitted`、`missing_delivery_defaults_to_steer`、`blank_delivery_is_a_400` | web/tests/prompt_delivery_validation.rs |
| clear_model/agent 互斥+NULL | `clear_model_and_clear_agent_null_the_columns`、`rollback_constructors_restore_value_or_clear`、组合互斥扩展 | store/tests/session_patch_conflict.rs |
| TOCTOU 回滚到 NULL | `post_agent_rolls_back_null_agent_by_clearing`、`post_model_rolls_back_null_model_by_clearing`、`post_{agent,model}_clears_*_when_capture_read_failed` | web/tests/agent_model_toctou.rs |
| handle 订阅者保留 | `resume_failure_keeps_handle_with_live_subscribers`、`resume_failure_removes_handle_with_zero_subscribers`、`resume_failure_keeps_handle_with_uncounted_receiver`、`release_subscriber_does_not_underflow_fresh_instance` | web/tests/handle_resume_failure_keeps_subscribers.rs + handle.rs |
| bash 守卫 4 类绕过 | `if_then_body_write_is_blocked`、`loop_do_body_write_is_blocked`、`case_pattern_body_write_is_blocked`、`env_with_flags_hiding_write_is_blocked`、`wrappers_with_valued_flags_hiding_write_are_blocked`、`time_stdbuf_setsid_wrappers_hiding_write_are_blocked`、`pipe_to_bare_shell_interpreter_is_blocked`、`pipe_to_stdin_convention_script_interpreter_is_blocked`、`find_file_writing_actions_are_blocked`、`sed_long_in_place_form_is_blocked` + 各 `…_stay_allowed` 对照，共 19 例 | session/src/bash_guard_bypass_regression.rs |
| verify 滑窗配对 | `window_drops_leading_orphan_tool_result`、`window_strips_trailing_unanswered_tool_use_blocks`、`many_pairs_straddling_the_boundary_stay_well_formed` 等 6 例 | session/src/autopilot/verify.rs |
| resume 回放过滤 | `resume_and_replay_skips_task_with_persisted_result`、`…_skips_task_dispatched_below_handoff_boundary`、`…_below_compaction_boundary`、`…_replays_only_the_unanswered_visible_task`、`pairing_helper_detects_duplicates_and_orphans` | session/tests/resume_replay_guards.rs |
| clear_context 计划门槛 | `act_mode_clear_context_uses_sentinel_not_fabricated_plan`、`act_mode_after_plan_inputs_still_preserves_plan`、`apply_clear_context_act_mode_does_not_fabricate_plan` 等 4 例 | session/tests/clear_context_regression.rs + control_cmd 单测 |

本轮新增测试 61 个。

- 全量回归（当次实跑）：`cargo test --workspace` → **2542 passed / 0 failed**（本轮开始基线 2464 passed / 0 failed；+78 ≥ 本轮新增 61，余量为并行会话在途 question 工具测试，亦全绿）
- 分套取证：llm lib 92 passed（http_date 9 / retry_delay 4 当次过滤实跑）；session lib bash_guard 族 61 passed（42 旧 + 19 新）；session integration resume_replay_guards 5 passed；web agent_model_toctou 6 / prompt_delivery_validation 4 / handle_resume_failure 3 passed；store session_patch_conflict 全绿
- clippy（当次实跑）：`cargo clippy --workspace --all-targets -- -D warnings` → Finished，零警告
- build（当次实跑）：`cargo build --workspace` → Finished
- 行数：新文件 bash_guard_bypass_regression.rs 329 ≤ 400、resume_replay_guards.rs 386 ≤ 400、handle_resume_failure_keeps_subscribers.rs 188 ≤ 400、prompt_delivery_validation.rs 143 ≤ 400；迭代文件均 ≤ 800（key_handler_tests.rs 的 rustfmt churn 已回退，799 ≤ 800；control_cmd.rs 1070 为 HEAD 既有存量、本批仅 +4）
- 提交前复核（submit gate 实跑）：`cargo test --workspace` → 2541 passed / 0 failed；`cargo clippy --workspace --all-targets -- -D warnings` → 零警告；`cargo build --workspace` → Finished

## Impact Surface
- **用户可见**：`Retry-After` 不再导致长达一天的挂起；`delivery` 拼写错误显式 400 而非打断 turn；model/agent 切换被 409 拒绝后 meta 不再残留新值；SSE 订阅不因后台 resume 失败而孤儿化；plan 模式 bash 写命令绕过面收窄；`/act_clear_context` 不再伪造计划；autopilot 判定与跨进程 subagent resume 不再因 provider 400 损坏 session
- **不影响**：`Store` trait 签名、`ChatStream` trait、drain 主循环时序；并行会话 question 工具在途改动未被触碰（resume.rs 的 question_hub hunk 完好）

## Related Docs
- agents/store/index.md：SessionPatch 互斥对补 model/agent + rollback 构造器一行（repair-on-touch）
