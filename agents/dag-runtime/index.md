# dag-runtime — 节点侧 DAG 执行运行时

`crates/dag-runtime`：被 claim 的 `DagClaimedRun` 在节点上的完整执行器。只被 `opencode-agent` 二进制链引用（VM/runc 重依赖不进 server）。

## 结构

- `runtime.rs` — `execute_run(RunDeps, run, cancel_rx)`：spec 复验 → run_started → JoinSet 调度（`MAX_CONCURRENT_STEPS=4`，ready 重算排挤）→ 每步 timeout + panic catch → 工件落盘 + step_done → 终态折叠（cancelled>error>done）→ `dag_status` 上报 + run_finished。cancel watch → CancellationToken 转发；blocked 依赖传播为 `Error("blocked: …")`。
- `step_io.rs` — `record_step`/`write_step_artifacts`/`mark_unfinished`（pub(crate) 记账辅助）。
- `dag_events.rs` — `RunEventSink`：无界 mpsc + 批量上传（≥8 条或 300ms，3 次退避后 warn-drop，终态前 flush）。
- `exec/agent.rs` — agent step = 节点本地真 session（ULID 会话、`上游步骤输出（JSON）`上下文头、json fence 提取 output.json、8KB 转录尾、取消优先级同 node 任务）。
- `exec/python/` — `execute_python_step`：默认内嵌 RustPython 0.5 VM（spawn_blocking；`Settings::install_signal_handlers=false` 防劫持宿主信号——否则 SIGCHLD 短暂 SIG_IGN 会偷走 waitpid 的子进程；StringIO 捕获 stdout/stderr；globals：`RUN_ID`/`STEP_DIR`/`context`；约定写 `STEP_DIR/output.json`）；`sandbox: runc` 时 fail-closed 走 OCI bundle。tests.rs 为内联测试模块。
- `sandbox/oci.rs` — 纯函数生成 OCI config.json（bind run 目录→`/workspace/context` rw、rootfs readonly、rbind 选项、mountpoint 预建、ociVersion "1.0.0"）+ `write_rootfs_template`（`opencode-agent dag prepare-rootfs` 的后端：纯目录脚手架，无网络下载）。
- `sandbox/runc.rs` — `runc_available`/`run_step`：并发排空 stdout、timeout → `runc kill` + 无条件 `delete --force`。

## 关键事实

- RustPython `with_init` 默认**不带 stdlib**（无 json/math，有 _io/itertools/posix）——python step 代码需手写 JSON 或走 runc rootfs 的完整解释器。
- runc rootfs 必须绝对路径、不可 symlink 中转；fixture 测试用 `DAG_TEST_ROOTFS` 指向名为 `rootfs` 的目录，否则跳过。
- 测试：lib 20（python 10 / oci 5 / runc smoke 1 / runtime+step_io 4）+ `tests/run_loop.rs` 4（对本地 axum stub uplink 的全流程）。
