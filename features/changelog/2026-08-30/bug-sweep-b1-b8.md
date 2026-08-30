# 全仓缺陷扫除：B1–B8 八项闭环（双路只读侦察 → 修复 → 全量回归）

## 背景

对 main（`1232cd4`）做双路只读侦察（session/todos/web/store 主链路 + llm/node/cli/tui/shellguard/core），三门基线全绿（build 干净 / clippy 0 告警 / 3729 passed）但确认 8 条位于测试盲区的逻辑缺陷：1×P1、2×P2、5×P3。本轮全部修复收敛，shellguard `/tmp` TOCTOU（静态分类固有边界，需执行环境闭环）明确不属本轮。

## 实现

- **B1（P1）todos 验收门禁恒假**（`todos/src/execution.rs`）：`run` 的回调原为空操作 `|_| {}`，而 runner 事件落库完全依赖回调方推进 `EventSink`——TODO session 的 ToolStart/ToolEnd 从不落库，`events_after` 恒空 → `required_tool_calls` 全 unmatched，带工具验收合同的 TODO 恒 Failed。修复：回调实时收集 `SessionEvent`（`Arc<Mutex<Vec<_>>>` + run 后 `Arc::into_inner` 回收）直接喂 `evaluate_gate`，同时经 `spawn_event_flusher(Some(store), sid)` 落库对齐 web drain 观测面（run 结果先收、`drop(sink)` 后 **await flusher** 保证终批落库，失败仅 warn）；删除死的 `last_event_seq`/`events_after`/`from_sse` 重解码块。
- **B2（P2）sandbox 分类 cwd ≠ 执行 cwd**（`shellguard/src/lib.rs` + `session/src/bash_guard.rs` + `runner/execute.rs`）：`classify` 按进程 cwd 分类而 bash 实际用 per-call workdir 执行，进程 cwd∈/tmp 时相对路径写可绕过写拦截。修复：shellguard 新增 `classify_in(cmd, cwd)`（`classify` 变薄包装），bash_guard 新增 `classify_with_dir`、`gate` 增加生效 workdir 参数；调用点按与 `tools::bash::execute` 完全一致的规则解析 workdir（`input["workdir"]` 缺省 session `working_dir`）。契约：**分类 cwd ≡ 执行 cwd**。
- **B3（P2）心跳预算失配**（`node/src/uplink.rs` + `runner.rs`）：控制面 READ_TIMEOUT=120s 内联串行 await vs server `STALE_AFTER_MS=20s`——一次慢心跳就把活节点误判 lost，running 任务被错误收敛 error、工作白做。修复：心跳走独立 `HEARTBEAT_TIMEOUT=5s` 短超时（`with_heartbeat_timeout` 测试注入口子；最坏静默间隙 ≈ 5s 超时 + 5s tick < 20s，约 2× 余量，tick `Skip` 行为天然提供失败后立即重跳）；三处 `handle_control` 全部 `tokio::spawn` 脱离 tick 关键路径（`Inflight` mutex 下原子 `insert_if_absent` 兜并发去重）。
- **B4（P3）SSE 永不发 `id:`**（`web/src/api_events.rs` + `sse_nodes.rs`）：`evt.seq` 为 `Some` 时补 `.id(seq.to_string())`，文档宣称的 Last-Event-ID 重连从死路径变可用。
- **B5（P3）`now_ms - i64::MIN` 溢出**（`core/src/auth_sig.rs::verify`）：改 `saturating_sub().saturating_abs()`，debug 构建单请求 panic DoS 消除，release 饱和绕窗同步封死。
- **B6（P3）重放缓存剪枝边界**（`web/src/auth_sig_mw.rs`）：剪枝 `>` → `>=`，与 `verify` 含端点窗口对齐，窗口最后一毫秒不再可重放两次。
- **B7（P3）收敛广播失败中断 500**（`web/src/api_nodes.rs::list_nodes`）：sweep 已提交后单条 `emit_closure` 失败由 `return error_500` 降级为 warn + 继续，error 终帧不再永久丢失。
- **B8（P3）SSE 跨 chunk CRLF 分裂**（`llm/src/sse.rs`）：`self.buf` 改存原始字节，帧界扫描在原始缓冲上做（终结符集 `["\r\n\r\n","\n\r\n","\n\n","\r\r","\n\r"]`，同偏移最长优先），CR 规范化延后到帧切分之后逐帧进行——`\r` 结尾 chunk 不再被提前规范化成假边界。

## 测试（功能 → 测试名映射）

- B1 端到端正例：`todos/tests/boundary_guards.rs::required_gate_passes_when_tool_really_ran`（mock 真跑 `bash echo hi` 工具轮 → gate 通过 → todo Passed → workflow Completed，且断言 session 事件已落库含 `tool_start`/`tool_end`）；M6 负例 `acceptance_correction_reasks_on_failed_gate` 语义保留（工具没跑=真失败）。
- B2：`shellguard/src/classify_in_tests.rs`（`relative_write_is_released_when_cwd_is_tmp` / `relative_write_is_blocked_when_cwd_is_not_tmp` / `cwd_alone_flips_the_verdict_for_the_same_command`）；session 单测 `gate_judges_bash_against_the_call_workdir_not_the_process_cwd`（进程 cwd=仓库目录反证）、`classify_with_dir_resolves_relative_paths_against_the_given_cwd`；集成 `sandbox_mode_releases_relative_write_in_tmp_call_workdir` / `sandbox_mode_blocks_write_in_plain_call_workdir`。
- B3：`node/tests/heartbeat_budget.rs`（`hung_heartbeat_times_out_within_injected_budget` / `heartbeat_recovers_after_timeout` / `runner_keeps_beating_after_a_timed_out_beat`，stub 支持有界 park 挂起注入）。
- B4：`web_list_events.rs::persisted_frames_carry_seq_as_sse_id`、`nodes_api.rs::node_task_events_frames_carry_seq_as_sse_id`。
- B5：`core/src/auth_sig.rs::extreme_timestamps_reject_without_overflow`（i64::MIN/MAX 双向 + 窗口内 Ok 对照）。
- B6：`auth_sig_mw` 单测 `window_edge_entry_survives_prune_so_replay_is_caught`（注入 now 钉死 exp==now 边界）+ 集成 `auth.rs::window_edge_timestamp_replay_is_rejected`（200→409）。
- B7：`store_error_surfacing.rs::lost_node_sweep_emit_failure_does_not_500_list`（ErrorStore 按 session 精确注入 `append_events` 失败，断言仍 200 且任务已终态收敛）。
- B8：`llm/src/sse.rs`（`drain_keeps_cr_pending_across_chunks` / `drain_splits_on_bare_cr_boundary_across_chunks` / `drain_splits_on_lf_line_plus_cr_empty_line_across_chunks` / `drain_prefers_longest_terminator_at_same_offset`）。

## 回归门

- `cargo build --workspace` 干净；`cargo clippy --workspace --all-targets -- -D warnings` 0 告警；`cargo test --workspace --no-fail-fast` → **3750 passed / 0 failed**（242 个 test result 面；基线 3729 + 新增 21）。
- 行数 gate：迭代文件最大 `runner/execute.rs` 799 ≤ 800；新增文件最大 153 行 ≤ 400。

## 边界

- shellguard `/tmp` TOCTOU 不属本轮（静态分类固有边界，需执行环境闭环）。
- B6 的 e2e 用例取 `now - WINDOW + 2s` 余量（中间件时钟必然前进，字面最后一毫秒写法必 flaky）；精确边界由可注入 now 的 `check_and_record` 单测钉死。
- B3 后弱网心跳失败只等下一 tick 补跳（判失联由 server 侧 20s 窗口 + 2× 余量兜底），不引入主动重试风暴。
- `crates/cli/src/display.rs` 存在 HEAD 既有 rustfmt 漂移，不属本轮改动集，未触碰。
