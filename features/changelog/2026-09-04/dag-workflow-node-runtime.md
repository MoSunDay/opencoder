# DAG workflow：节点侧执行运行时 + 三二进制拆分

日期：2026-09-04 ｜ 提交：(working tree)

## 动机

原 `opencode daemon` 单入口同时承载 web 服务与节点 worker，且 DAG 调度若放 server 侧会把 VM/runc 重依赖拖进服务进程。本次把执行面整体下沉到节点：server 只做 def/run 管理、claim 分发与事件汇聚；被 claim 的 run 在节点本地完整执行（agent step 走真 session，python step 走内嵌 RustPython VM 或 runc OCI 沙箱）。

## 变更

### P0 — 三二进制拆分

- `crates/server`（bin `opencoder-server`，替代 `daemon --server`）：web 薄入口，不链接 VM/runc。
- `crates/agent`（bin `opencode-agent`，替代 `daemon --client`）：fleet worker 入口，默认挂 DAG hook（`--no-dag` 关闭），新增离线子命令 `dag prepare-rootfs --out DIR`。
- `crates/cli`：`daemon` 子命令仅打印迁移指引；根二进制 `opencode` 保持 CLI/TUI/headless。

### P1/P1.5 — 域层与持久化

- 新 crate `crates/dag`：spec/validate（环检测、slug、缺依赖）、ready_steps/run_outcome/render_context、`/workflow/<run_id>/<step>/{output.json,output.txt,meta.json}` 工件契约、protocol DTO（LOCKED）。
- store schema **v16**：`dag_defs`/`dag_runs`/`dag_events` 三表 + 索引；claim 单活跃/节点、FIFO `(created_at,rowid)`、BEGIN IMMEDIATE CAS；失联收束复用 converge 机制（running/cancelling → error("node lost")）。

### P2 — server 面

- `POST /api/dag/defs`、`GET /api/dag/defs[/:id]`、`DELETE /api/dag/defs/:id`、`POST /api/dag/defs/:id/dispatch`、`GET /api/dag/runs[/:id]`、`POST /api/dag/runs/:id/cancel`。
- 节点面：`GET /api/nodes/dag/claim?node_id=`、`POST /api/nodes/dag/runs/:rid/events`、`POST /api/nodes/dag/runs/:rid/status`；心跳应答 `cancel_run_ids` 捎带取消；终态上报补写合成 `run_finished`。
- SSE：`GET /api/dag/runs/:id/events`（id=seq，Last-Event-ID 续传）。

### P3 — SPA

- 「DAG」面板：defs 编辑/校验（specValidate）、runs 表、run 详情事件时间线、@xyflow + dagre 拓扑图。

### P4 — 节点执行运行时

- 新 crate `crates/dag-runtime`：`execute_run` JoinSet 有界调度（并发 4）+ 每步 timeout + panic catch + blocked 依赖传播；事件批量上行（≥8 条/300ms，3 次退避）；终态折叠 cancelled>error>done。
- agent step：节点本地真 session（ULID 会话、上游输出 JSON 上下文头、```json fence 提取 output.json、8KB 转录尾）。
- python step：默认内嵌 RustPython 0.5（**无 stdlib**；`install_signal_handlers=false` 防宿主信号劫持）；`sandbox: runc` 走 OCI bundle（bind run 目录→`/workspace/context` rw、rootfs readonly、fail-closed）。
- node crate：`DagHook` 扩展点 + idle claim 轮询 + per-run heartbeater + uplink 三方法。

## 测试清单

- `crates/dag`：18 unit（spec/domain/transitions/artifacts/protocol）。
- `crates/store`：`tests/dag_store.rs` 9。
- `crates/web`：`tests/dag_api.rs` 5、`tests/dag_api_sse.rs` 4、`tests/dag_e2e_flow.rs` 2（真 server + 真 runtime：SSE 帧/工件/终态/取消）。
- `crates/dag-runtime`：lib 20（python 10 / oci 5 / runc smoke 1 / runtime+step_io 4）+ `tests/run_loop.rs` 4。
- `crates/node`：`tests/runner_dag.rs` 2（签名 uplink claim + 心跳取消）+ 既有 26 项回归。
- `crates/agent`：2（含 `dag prepare-rootfs` 参数解析）。
- 根目录进程级 smoke：`daemon_smoke.rs` 1、`running_mode_switch_e2e.rs` 1、`nodes_smoke_proc.rs` 2（已迁移到新二进制；`tests/support` 用 CARGO_BIN_EXE_opencoder 同目录推导兄弟二进制）。
- SPA：dagPanel/dagProjection 套件（293 passed，P3）。
- runc 实测：runc 1.1.12 + python rootfs fixture 下 `runc_step_smoke` 绿（无 fixture 时自动跳过，`DAG_TEST_ROOTFS` 可指定）。

## 运维注意

- 节点执行 DAG 需 `--workflow-root`（默认 data 目录下）；runc 模式需先 `opencode-agent dag prepare-rootfs` + 自备 python 解释器树。
- 旧 `opencode daemon --server|--client` 调用方按 `daemon` 打印的指引迁移。
