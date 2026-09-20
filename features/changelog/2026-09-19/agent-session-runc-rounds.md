# agent 会话 runc 沙箱回合：run_mode: agent 的 kind=agent 执行全容器化

## 背景

- 引用卡 `run_mode`（`Agent` = 每回合只读 runc 沙箱）此前只在 DAG agent 步
  （`dag.agent_sandbox="runc"`）落地；`kind=agent` 执行仍无条件走宿主进程
  session 循环，与"节点执行面不可信负载必须沙箱化"的边界不符。
- 目标：目标卡钉 `Agent` 模式的 `kind=agent` 执行，每个回合是一轮 runc 容器
  （`/usr/bin/agent-session-runner`，与 DAG 步同一机制），宿主只做状态 staging
  与事件中继；builtin agent 与旧卡（无 `run_mode`）行为零变化。

## 变更（crates/worker）

- 新增 `src/workloads/agent_runc.rs`：
  - `session_uses_sandbox(root, agent)` 判定：builtin 名单一律 host；池内
    `meta.json` 的 `run_mode==Agent` 才进沙箱，缺卡/坏卡不翻转（fail-closed
    于 host 侧）。
  - `run_round`：复用 `super::agent::create_session` 保留宿主 store 会话行
    （SSE/控制台照常 attach）；空 prompt 的 relaunch 只回
    `Idle,{"session_id"}`；run root `<workflow root>/<id>/session` 种子
    messages.json 全量历史 + 本回合截断的 events.ndjson + prompt.txt，bundle
    写 `<workflow root>/bundles/agent-sessions/<id>`（容器 `ag-<Ulid>`，
    `ArgvStyle::Direct`，agents 池 ro 挂 `/workspace/agent`，env 注入
    OPENAI_BASE_URL/OPENAI_API_KEY/OPENCODER_MODEL/OPENCODER_STEP_* 与
    how_append 对）；`tokio::select!` 容器流 vs 300ms 轮询 tail。
  - 退出映射：0 + 容器 error 帧 → `agent execution failed`；非 0 → 带
    2KiB 尾巴的 `runc agent session exited`；cancel → `Cancelled`（结果与
    原生路径逐字段一致，含 Idle 的 how_append warn-only +
    `transcript_tail`/`extract_output_json_from` 产出 output_text/json）。
    messages 增量折回 store 有压缩收缩护栏（`len > prior.len()` 才切片）。
  - 准入 `preflight` fail-closed：runc 可执行、`<workflow root>/rootfs` 真实
    目录、LLM key；`create.rs` prepare 接受时与每轮 run_round 双检。
- 新增 `agent_runc/events.rs`：ndjson 事件尾随（只消费完整行、部分行留待
  下次、坏行跳过仍计 offset、sidecar 帧丢弃不入库、error 帧取最后一条），
  记录形状与 web 事件 sink 一致（`SessionEventRecord`）。
- `operations/command.rs`：沙箱会话 POST 拦截——prompt 在容量许可段内 staged
  进执行 input（排队克隆同样携带）并回 202 accepted（轮次由 launch 的
  run_round 执行）；活跃回合再 prompt 409；steer/queue/compact/handoff 409
  （v1 不支持）；GET 全部保持原生（读 store）。
- `operations/queue/mod.rs`：排队 prompt 重放同样拦截 staged——否则原生重放
    会经宿主 web app 启动一个 HOST 回合（fail-open 漏洞）。
- `workloads/agent.rs`：分派块（`declared_how_append` 之后）按
  `config.agent.agents_dir` 判定转 `run_round`；Operator/Maintenance 不受影响。

## 测试

- `workloads/agent_runc/tests.rs`（6）：沙箱选择（builtin/池卡/坏卡/缺省
  act）、parse_event_lines 三态（完整+部分尾、坏行+error 捕获、sidecar 丢弃）、
  `drain_events` 批量入库与 offset 推进。
- `operations/sandbox_session_tests.rs`（4）：steer/queue/compact/handoff 409、
  无 runc 节点 fail-closed（prepare 报 rootfs/runc 不可用）、排队沙箱 prompt
  重放为沙箱回合而非宿主回合、非沙箱会话保持原生 prompt 路径。
- 回归：`cargo test -p opencoder-worker` 全绿（34 套件 171 通过 0 失败，lib
  74 通过 1 忽略）；`cargo check --workspace` 零告警。容器内全链路由根包
  `tests/dag_e2e/agent_runc.rs` 既有真实 runc 用例口径覆盖（无 runc SKIP）。
- 全量门（`cargo test --workspace --lib` 21 套件 ok + `--tests --no-fail-fast`
  405 结果行，SPA vitest 854/854）：除下述先存红外全部 ok；本特性与并行
  schedule 迭代零新增失败（隔离复测失败集与 HEAD `9513d000` 逐用例一致）。
- 先存红（HEAD `9513d000` 即失败，非本迭代引入，根因为 `225718da` lane
  隔离把 `worker/service.rs` 的节点索引发布收窄为 `agent-` 前缀 +
  `include_subagents:false`，内部/成员会话不再进执行索引）：
  - `dag_e2e flow::dag_spec_dispatch_runs_wasm_and_agent_steps_to_done`（relay
    按子会话 id GET 404 "execution id not found"）
  - `team_e2e flow::team_topic_runs_to_completion_with_turn_ledger`、
    `web dag_e2e_flow claimed_run_executes_and_converges_done_on_the_server`
  - `worker workloads team_members_execute_locally_with_capability_prefixes`
    （`member-` 前缀索引条目缺失）
  - `worker platform workloads::ordinary_team_stays_on_one_node… /
    team_multiround_consensus…`（同族）
  修复方向（择一，归 lane 隔离归属方决策）：恢复子会话索引发布但保留
  控制面 `visible_chat_session` 泳道过滤；或改各断言走父执行 detail/event
  路由。全量日志中的 `dag_team_loop` 失败为双回归并发竞争抖动（隔离即绿）。
- 进程级 e2e（O6，`tests/operator_e2e/agent_sandbox.rs`，真 fleet 二进制）：
  无沙箱运行时准入 fail-closed、runc 全回合契约（result/output、
  meta/messages/SSE、容器内池解析 SOUL 标记、bundle args/env/ro 挂载、
  runner 产物、follow-up prompt 二轮续会话）、`run_mode: operator` 卡保持
  宿主循环（无 runc 也能跑）。
  - `sandbox_agent_session_preflight_fails_closed`
  - `sandbox_agent_session_runs_and_continues_in_runc`（无 runc SKIP）
  - `operator_mode_card_stays_on_host`
  - `cargo test -p opencoder --test operator_e2e` → 9 passed / 0 failed

Commit: 工作区未提交（与 schedule-form-and-dag-args 并行迭代共享工作树；基线 HEAD 9513d000）
