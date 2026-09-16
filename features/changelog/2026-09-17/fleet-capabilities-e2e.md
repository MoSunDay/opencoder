Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# Fleet 能力根级进程级 e2e 套件（todos/team/brain 三面）

为 fleet 三大业务能力新增三个根级进程级 e2e target：`tests/todos_e2e/`（T 系列）、`tests/team_e2e/`（M 系列）、`tests/brain_e2e/`（B 系列），全部走真 `opencoder-server` + `opencoder-agent` 二进制、真 WebSocket 节点注册与真实 HTTP，由脚本化 loopback LLM stub 驱动。`tests/support/llm_stub.rs` 同步扩展，根 `Cargo.toml` 零改动（各 target 以 `#[path="../support/mod.rs"]` 挂载共享 support）。

## stub 扩展（tests/support/llm_stub.rs）

- `Script::Dynamic(Arc<dyn Fn(&Value)->String+Send+Sync>)`：闭包收解析后的 OpenAI 请求体，按内容判别回复（回声式契约的杠杆）；`Script` 改为 `Clone`（Arc 共享同一闭包，可一次 spawn 多份同闭包条目）。
- `/embeddings` 路径 canned 响应（`data[].embedding` 定长向量），不记入 requests、不消耗脚本——brain 能力卡经同一 base_url 嵌入时保持记账纯净。
- `read_http_request` 返回 (path, body)；既有 FIFO/`Hold`/`Fail`/`EXTRA_REPLY` 兜底语义不变，`running_mode_switch_e2e` 向后兼容编译通过。

## 各面覆盖（现场实测契约已固化进断言）

- T1：模板 v1 固化 → run 202 → 父/子会话 4 次 FIFO → `todo_candidate_ready`/`todo_accepted`/`workflow_completed`，item passed、`<node-state>/todos/<id>/execution.json` 磁盘断言。
- T2：`Hold` 挂起 → `/interrupt`（200 `cancelling`，durable 中断异步落地）→ kill+respawn（node_id 不变）→ `/resume`（与重启恢复竞态时 409 属正常）→ done；SSE 挂起折叠事件在 `workflow_interrupted`/`runtime_error` 间竞态，断言取先出现者。
- T3：子 LLM `Fail(400)` → `todo_execution_failed` → todo Failed（max_attempts=1）→ 父决策吃 EXTRA_REPLY 连环解析失败（PARSE_RETRIES+DISPATCH_CORRECTION_RETRIES 共 5 请求）→ runtime_error → workflow suspended，`/api/executions/:id/todo-items` 的 `last_error` 携带 outage 文案。
- M1：能力注册（`POST /api/brain/capabilities` 201）+ 绑定（`PUT /api/brain/capabilities/:id/target`）→ pinned 团队定义冻结 `capabilities` → member prompt 带 `你的能力：{summary}` 前缀 → 四段 chat 决策 → finished topic + 1-based turn 台账 + 磁盘 `team.json`（Team/Topic 两层）+ member 执行索引（`member-` 前缀）。
- B1：`POST /api/brain/plan-defs` 固化 v1（builtin 能力校验经 resolve）→ `POST /api/brain/runs`（fixed）202 → 子 agent 会话真转录（含 `Named output descriptions` 契约与 envelope 回复）→ route 决策回显 prepared receipt → phase=completed、实例 id `agent-<40hex>`、watermark>0 → 重放 202 且模型调用数不变 → events-page 非空。
- B2：`POST /api/executions kind=brain` 旁路 409（"registered immutable plan versions"）；route 模型回异样 receipt → phase=blocked（error 含 "foreign receipt"，实例本身仍 succeeded）→ `/commands cancel` → cancelled + execution cancelled → 终态再命令 500 拒绝（"run is terminal; create a new run to execute again"）。

## 现场实测要点（已写进测试注释，防回归误判）

- brain 子 agent 会话完成后有一次 best-effort **标题生成调用**（`RequestPurpose::Title`，messages 原样回放、无 system preamble），插在子会话与 route 调用之间——FIFO 槽位会被吃掉，stub 必须按内容判别（`Named output descriptions` vs RouteContext `receipt`），这也是 worker 单测桩 `GraphClient` 的同一思路。
- inspect 文档里 agent 会话的 `session.messages` 是 base64 分块页；解码转录走 `GET /api/sessions/{id}`（返回明文 messages）。
- brain `events-page` 返回 `{events,more}`，无 `finished` 字段。

## 测试清单

| 场景 | 测试 | 位置 |
| --- | --- | --- |
| T1 TODO 全流程 | `todo_template_runs_to_completed_with_passed_item` | `tests/todos_e2e/flow.rs` |
| T2 中断-重启-恢复 | `interrupt_survives_node_restart_and_resumes_to_done` | `tests/todos_e2e/lifecycle.rs` |
| T3 子失败折叠 | `child_model_failure_suspends_workflow_with_failed_todo` | `tests/todos_e2e/lifecycle.rs` |
| M1 team 话题与 turn 台账 | `team_topic_runs_to_completion_with_turn_ledger` | `tests/team_e2e/flow.rs` |
| B1 固定图完成+收据路由+重放 | `fixed_plan_run_completes_through_child_session_and_receipt_route` | `tests/brain_e2e/flow.rs` |
| B2 旁路守卫 | `raw_brain_submissions_are_rejected_in_favor_of_plan_versions` | `tests/brain_e2e/lifecycle.rs` |
| B2 异样收据 blocked→cancel | `foreign_receipt_blocks_and_cancel_folds_to_terminal` | `tests/brain_e2e/lifecycle.rs` |
| stub 扩展向后兼容 | `real_server_rejects_running_mode_switches_until_idle`、`real_server_clear_context_executes_preserved_plan_in_act` | `tests/running_mode_switch_e2e.rs` |

## 回归取证

- 构建：`cargo build --workspace --bins` 通过（sibling_bin 前置：server/agent/cli 同目录）。
- 新 target 单独跑：`cargo test --test todos_e2e`（3 passed）、`cargo test --test team_e2e`（1 passed）、`cargo test --test brain_e2e`（3 passed），均零 warning。
- 全量回归：`cargo test --workspace` → 409 个测试目标全部 `test result: ok`，0 failed；根级 e2e 全绿（todos_e2e 3、team_e2e 1、brain_e2e 3、dag_e2e 5、operator_e2e 5、running_mode_switch_e2e 2）。
- clippy：`cargo clippy --test brain_e2e --test todos_e2e --test team_e2e` 零警告；新增文件均 <400 行、根 `Cargo.toml` 零改动。
