Commit: e797e184412ac6df34cd5e8189634e1361945362

# Operator + DAG 进程级 e2e 套件（O1–O4 / D1–D3 / D5 条件）

## 背景

Operator（会话编排面）与 DAG（结构化执行面）此前只有 mock/内嵌层测试，缺少「真二进制 + 真 HTTP/SSE + 真 wasm」的进程级固化：`opencoder-server` 与 `opencoder-agent` 以子进程拉起、控制面走真实 HTTP/SSE、LLM 用 loopback 确定性桩、wasm 用 wat 现场编译。本次不改生产代码，把已核实的 wire 契约固化为两个根包 layer-2 套件（`cargo test` 即跑，零凭据）。

## 变更

- 新增 `tests/support/`（共享层，由既有套件抽取）：`llm_stub.rs`（OpenAI 兼容流式桩：FIFO 脚本 / `Hold` 挂起 / `Fail` 非重试状态 / 请求体记录）、`http_util.rs`（手写 Bearer JSON 客户端 + `id:`/`event:`/`data:` SSE 解析 + 50ms deadline 轮询）、`fleet_proc.rs`（fleet 拉起/就绪等待/RAII 终止 + 失败日志尾 panic）。
- `tests/running_mode_switch_e2e.rs` 重构为消费 `support::llm_stub`（行为不变，去重 ~120 行）。
- 新增 `tests/operator_e2e/`（O1–O4）与 `tests/dag_e2e/`（D1–D3、D5），场景矩阵见下表。全部断言打在公开 wire 契约上（202/200/400/403/404/409 状态码与错误文案逐字），节点侧仅断言 LOCKED 产物契约（`meta.json`/`output.txt`/`session.json`/`input.json`/`_modules`）。
- 根 `Cargo.toml`：`wat = "1.254"` 进 workspace deps + root dev-deps（唯一 manifest 改动）。

## 场景矩阵

- O1 `flow` — `POST /api/sessions` 一次创建会话 + operator 执行；真实 drain 后终态为 `idle`（非 terminal）、result == `{"session_id"}`、inspect 投影 `session.meta.title == "Operator"`、转录含 preamble 包裹的 prompt 与桩回复、SSE 回放 `text_delta{text}` 至 `stream_end{finished:true}`。
- O2 `relay_sse` — `/api/sessions/:id/*` 中继（command action `http`）：后续 prompt 二次 drain（`driver_ensured`）、GET 读活跃状态、穿越/空 id 400「invalid session path」、非白名单方法 400「invalid session operation」、未知会话 404「execution id not found」。
- O3 `gating` — `POST /api/users` 铸造 `oc_` 令牌；非管理员可跑 operator 执行全程（提交/inspect/读会话）但 `/api/users` 403「admin role required」、非 operator 提交 403「non-admin roles may only submit operator executions」；admin 全量面不受限。
- O4 `lifecycle` — `Script::Hold` 挂住 drain 后 relay interrupt → 执行折叠为 `cancelled` 且事件流含 `status:interrupted`；agent 重启后旧会话转录仍在、新会话正常 drain（libsql 持久化跨进程恢复）。
- D1 `flow` — dag defs 保存/dispatch（走真 CLI：exit 0 + 纯 JSON stdout）/终态 done/progress 2 done/步产物与 meta/agent 步子会话经中继 GET（title `dag/<run>/<step>`）/运行事件 `run_started→…→run_finished{status:done}`（`stream_end` 为传输尾帧）/CLI `dag runs get` 兼容视图。
- D2 `wasm_pool` — 版本池 REST（201/409/400/404）、v1→v2→回滚 v1 的 current 指针、`echo@v2.wasm` 显式 pin 压过 current、冻结库 `_modules` 双 token 并存、`/versions/:v/wasm.bin` 二进制下载逐字节一致、`dag.wasm_dir` 覆盖 + `/api/dag/wasm/nfs` root 回显。
- D3 `cancel_fail` — 运行中 cancel（phase `cancelling`）→ 折叠 `cancelled`、progress `cancelled==2`、两步 meta.json 保留（`after` error == "run cancelled"）；agent 步桩回 400（非重试集）→ run `error` + progress.execution_error 非空。
- D5 `runc` — preflight 契约：无 rootfs 目录 400 提及 rootfs；搭好 rootfs 脚手架后无 `runc` 二进制 400 提及 runc（并 SKIP 沙箱运行段）；有 `runc` 则 dispatch 202 接受、运行时因脚手架无 wasmtime fail-closed 报 error。

## 关键发现（记录）

- inspect 文档的 `session.messages` 是 base64 字节分块（`encoding:"base64"`），明文转录断言必须走中继的原生 `GET /api/sessions/:id`（`{id,meta,harness,messages,draining}`，已解码）。
- SSE 事件流在业务终帧（`run_finished`/`done`）之后还有传输尾帧 `stream_end{"finished":true}`；「最后一帧」断言必须先滤掉它。
- DAG cancel 路由是 `POST /api/dag/runs/:id/cancel`（200 `{ok,phase}`），不是 `/api/executions/:id/cancel`；live run 返回 phase `cancelling`。
- text_delta 事件 payload 是 `{"text":…}`；事件分页 `finished` = 非 draining 且不在 active 表，故 idle 会话的 SSE 会正常收尾。

## Impact Surface

- 新增 `tests/support/{llm_stub,http_util,fleet_proc}.rs`、`tests/operator_e2e/{main,flow,relay_sse,gating,lifecycle}.rs`、`tests/dag_e2e/{main,fixtures,flow,wasm_pool,cancel_fail}.rs`
- 修改 `tests/running_mode_switch_e2e.rs`（改用共享桩）、根 `Cargo.toml`（wat dev-dep）
- 生产代码零改动

## 测试覆盖

| 契约 | 场景 | 测试 |
| --- | --- | --- |
| operator 创建→drain→idle + result/转录/SSE 回放 | O1 | `operator_e2e::flow` |
| relay 后续 prompt / 守卫 400 / 未知 404 | O2 | `operator_e2e::relay_sse` |
| 非 admin 角色门禁（正向 + 403 文案） | O3 | `operator_e2e::gating` |
| admin 全量面保留 | O3 | `operator_e2e::gating`（admin_keeps_everything） |
| drain 中断 → cancelled + 事件流 + 重启恢复 | O4 | `operator_e2e::lifecycle` |
| spec 保存/dispatch/wasm+agent 步/产物/progress/事件/CLI | D1 | `dag_e2e::flow` |
| 版本池 REST + 指针回滚 + 显式 pin + 冻结库 + 下载 | D2 | `dag_e2e::wasm_pool` |
| 运行中取消折叠 + 未启步标记 + 产物保留 | D3 | `dag_e2e::cancel_fail` |
| agent 步非重试失败 → run error | D3 | 同上 |
| runc preflight 契约（无 runc 时跳过运行段） | D5 | `dag_e2e::runc_preflight_contract_or_skip` |
| base64 编码参考向量 | fixture | `dag_e2e::fixtures::tests` |

## 测试与回归证据

| 目标 | 结果 |
| --- | --- |
| `cargo test -p opencoder --test operator_e2e` | 5 passed（1.64s） |
| `cargo test -p opencoder --test dag_e2e` | 5 passed（0.61s） |
| `cargo test -p opencoder --test running_mode_switch_e2e`（共享桩重构回归） | 2 passed |
| `--test daemon_smoke` / `--test nodes_smoke_proc` | 1+1 passed |
| `--test responses_cli` / `--test skill_seed_startup_wiring` / `--test tui_exit_restore_e2e` | 1+2+4 passed |
| `cargo clippy -p opencoder --tests -- -D warnings` | 零告警 |
| `cargo test --workspace --no-fail-fast --locked` | 5,222 passed / 0 failed / 7 ignored（随 e797e184 提交） |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 零告警（随 e797e184） |
