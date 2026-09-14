Commit: 2686d40a267436adb555041fc8154fc9bb454574

# DAG 单步执行记录：节点侧数据生产

## Context

DAG 步骤的执行痕迹此前只有两类：`<run>/<step>/` 下的静态工件（`meta.json`/`output.txt`/`output.json`，步骤结束才落盘）与经 Uplink 上报的 run 事件流。节点查询 API（`worker::operations::query::{dag_steps,dag_step_events}`）需要在**步骤运行中**就能回答两个问题：这一步的子会话是谁（供远程接管/查看完整对话），以及这一步到目前为止输出了什么（供分页拉取的增量日志）。两者都要求节点侧先生产出稳定的数据，契约 LOCKED：`session.json` 实时指针、`meta.json.session_id` 终态记录、run 会话上带 `payload.step` 的 `step_output` 事件行。

## Change Summary

- **契约纯函数**（`crates/dag/src/artifacts.rs`）：新增 `session_file`（`<step>/session.json`，复用已校验的 `step_dir`）、`session_value`（`{"session_id":"<ulid>"}`，正是查询端读到的形状）、`parse_session_id`（宽容解析：缺失/畸形返回 `None`）、`meta_value_with_session`（`meta.json` 增加可选 `session_id`；`meta_value` 委托它并传 `None`，旧文件与旧调用方零改动）。
- **实时会话指针**（`crates/dag-runtime/src/step_io.rs` + `src/exec/agent.rs`）：`write_session_artifact` 在 agent 步骤**创建子会话成功后立刻**写 `session.json`（`create_dir_all` + `checkpoint::write`），失败只 `warn!`——`meta.json.session_id` 仍是终态兜底，工件 IO 永不失败步骤；`write_step_artifacts` 改为把 `StepResult.session_id` 写进 `meta.json`。
- **步骤输出落库**（新增 `crates/dag-runtime/src/step_log.rs`）：`StepOutputLog` 用无界通道 + 独立泵任务把 stdout/stderr 增量写成 `SessionEventRecord{session_id: run_id, kind: Step, sse_kind: Some("step_output"), payload:{step,stream,text,at_ms}, ts: at_ms, seq: None}`；300ms 窗口或 4KiB 累积触发批量 `append_events`，单条 `text` 上限 8KiB（按字符边界切分，跨 push 的 UTF-8 残片按流分别缓冲），`close()` 幂等尾冲刷，追加失败只 `warn!` 丢弃（镜像绝不阻断步骤）。`tee_reader` 供管道式执行器旁路镜像。
- **执行器接线**：wasm in-process 的 `SharedSink` 在限流判定**之前**镜像每个字节（`exec/wasm/in_process.rs`）；`exec/wasm/mod.rs` 新增 `execute_wasm_step_logged(ctx, cancel, output)`，旧入口委托 `None`；`sandbox/runc.rs` 新增 `run_step_streamed`，把容器 stdout/stderr 包成 tee reader 后交给有界收集器（`run_step_cancellable` 委托 `None`，`worker::brain::container` 调用方不变）；`runtime.rs` 的 wasm 分派创建/关闭 `StepOutputLog`，成功、超时、取消、报错四种终态都先尾冲刷再返回。
- **runner 事件补齐步骤名**（`exec/runner/events.rs`）：写进 run 会话的行统一带 `payload.step`（`record(session, event, step)` + `with_step`，非对象 payload 原样透传），与 `runner_stage` 一致——messages 表没有步骤维度，查询端只能靠 payload 过滤。
- **外键兜底**（`runtime.rs`）：调度前 `ensure_run_session` 确保 run 会话行存在（`INSERT OR IGNORE` 语义，节点 workload 已建则无副作用），否则 `step_output` 会因外键约束被整轮静默丢弃；失败只 `warn!`，run 自身的事件/状态上报不依赖 Node Store。

## Impact Surface

- `crates/dag/src/artifacts.rs`（契约纯函数 + 测试）
- `crates/dag-runtime/src/{step_log.rs(新),step_log_tests.rs(新),lib.rs,step_io.rs,runtime.rs}`
- `crates/dag-runtime/src/exec/{agent.rs,wasm/mod.rs,wasm/in_process.rs,wasm/tests.rs,runner/events.rs}`、`src/sandbox/runc.rs`
- 测试：`crates/dag-runtime/tests/{run_loop.rs,runner.rs}`
- 消费端（另属 worker 任务，本改动不改其代码）：`worker::operations::query::{dag_steps,dag_step_events}`

## Notes / Compatibility

- `meta.json` 的 `session_id` 是**可选**字段：历史工件、wasm 步骤（无子会话）都解析为 `None`，查询端据此回落到 run 会话。
- `session.json` 只在 agent 步骤出现；它可能比 `meta.json` 更早存在（步骤仍在跑），也可能在步骤失败时成为唯一线索——查询端优先读它。
- 输出镜像是尽力而为：Node Store 不可用时只丢日志，不影响步骤结果、工件与 Uplink 上报。
- 与同日的 Uplink 实时日志（`exec/logs.rs` + `dag_events.rs` 的 `step_log` 事件）互补：一条走控制面推送 SPA，一条落 Node Store 供分页查询/断点续传。

## 测试清单（功能 → 测试名）

- 契约纯函数：`opencoder-dag::artifacts::tests::{meta_carries_an_optional_session_id, session_file_sits_in_the_validated_step_dir, session_value_roundtrips_through_parse_session_id, legacy_meta_json_still_parses_and_reports_no_session, meta_shape, paths_are_rooted_and_slug_gated}`
- step_output 记录形状/批量/切分/失败降级：`opencoder-dag-runtime::step_log_tests::{event_record_matches_the_locked_shape, chunks_split_on_char_boundaries, oversized_chunk_is_split_into_bounded_records, byte_threshold_flushes_without_close, window_flushes_a_small_batch_without_close, close_flushes_the_tail_batch, append_failure_is_warn_and_drop, split_utf8_and_invalid_bytes_survive_pushes, tee_reader_mirrors_piped_bytes}`
- wasm 输出镜像（单元）：`opencoder-dag-runtime::exec::wasm::tests::{guest_stdout_is_mirrored_as_step_output_events, guest_stderr_is_mirrored_with_its_stream_label}`
- 端到端（真 runtime + LibsqlStore）：`run_loop::wasm_step_output_is_mirrored_to_the_run_session`、`run_loop::single_agent_step_completes_and_reports_done`（`session.json` 形状 == `session_value`、与 `meta.json.session_id` 一致、会话在 store 中真实存在）
- runner 步骤名：`runner::runner_persists_codex_transcript_artifacts_and_recovers_without_reexecution`（run 会话所有行都带 `payload.step`，且存在 `runner_stage` 行）
