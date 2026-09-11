Commit: (working-tree, 基于 b465f440)

# 团队成员即 Agent：组队免填成员 ID/职责，能力快照由大脑固化

控制面组队链重塑：成员不再手填「成员 ID + agent 类型 + 职责文本」，队员就是 agent；
成员职责由大脑能力集的一句话 summary 代替，由控制面在 resolve 时固化进 pinned
definition，worker 与 SPA 均不再消费用户录入的 role。

## 协议（DTO LOCKED 之外的业务形状变更）

- `TeamMember { agent, capabilities?: [string] }`（删除 `id`/`role`）；成员身份 =
  agent 名，团队内必须唯一；`TeamDefinition.captain` 为 agent 名且 ∈ members。
- 最小用户输入 `{name, captain, members:[{agent}]}` 合法（`capabilities` 带
  `serde(default)`，不是用户录入）。validate 规则改为：agent 非空且唯一、captain
  归属成员、团队名规则不变；错误消息 "...the team name 'system' is reserved"
  （保留对象明确为团队名；agent 取名 `system` 合法）。
- 旧库存定义（含 id/role 字段）可反序列化（未知字段忽略），但 captain 为旧成员 ID
  的定义不再通过 validate——需重新保存为新形状。

## 控制面

- 新增 `GET /api/brain/agents`（admin-only，随既有 `/api/brain/*`）：`list_brain_capabilities`
  + 逐能力 `capability_target` 绑定，按 `kind==Agent` 分组，返回
  `{"agents":[{"agent","capabilities":[{"id","summary"}]}]}`（BTreeMap 稳定排序；
  未绑定/非 Agent 目标/坏绑定宽容跳过）。
- `resolve`（catalog.rs）Team 分支：validate 通过后把每个成员的 `capabilities`
  整体覆盖为该 agent 的绑定能力 summary 列表（无绑定 → 空数组，不报错），随
  assignment 下发。库存定义本身不落 capabilities——固化只发生在 pinned definition。

## Worker

- 成员键控改为 agent 名（dispatcher map、`MemberRef{node_id,name}`、captain 查找）。
- 成员 prompt 前缀 `你的职责：{role}` 删除；capabilities 非空时改
  `你的能力：{caps.join("；")}`，空则原样。session 标题 `{coordinator} / {agent}`。
- `TeamMeta` 物化用真实快照 `capabilities: m.capabilities`（原先拿 role 充当），
  队长规划 prompt 的 `擅长：{caps}` 自动拿到大脑固化的一句概括。

## SPA

- `fleet/teams.jsx` 重做：数据源 `GET /api/brain/agents`；表单 = 团队名 + 队长
  Select(showSearch) + 队员 Select(multiple, showSearch)；删除 Form.List 三件套；
  底部实时 roster（队长置顶 + 各 agent 一句 summary，无绑定显示「暂无能力画像」）；
  提交 `{name, captain, members:[{agent}]}`（captain 自动并入去重）。启动弹窗不动。

## 测试覆盖

| 功能 | 测试 |
| --- | --- |
| 协议 validate（唯一性/空白/外来 captain/最小形状/legacy 字段忽略） | `crates/core` `fleet::protocol` 单测 2 条 |
| `/api/brain/agents` 聚合（agent 入选、team/未绑定排除、admin-only：`role_gate` 断言 User/Root 均拒） | `control/tests/e2e/brain_api/agents.rs` + `role_gate.rs` 单测 |
| resolve 固化（绑定→成员 capabilities、无绑定留空、库存定义不被改写） | `control/tests/e2e/teams_dag_defs.rs` `team_resolve_freezes_member_capability_snapshots` |
| 团队 CRUD/校验/隐藏 system | `teams_dag_defs.rs` 全套更新（含校验错误消息断言） |
| worker 消费（能力前缀到 LLM、成员键控、多轮共识、cancel/harness/matrix） | `worker/tests/{workloads,harness/*,platform/*,harness_matrix}` |
| SPA 表单（搜索选 agent、roster、提交形状、启动幂等重试） | `fleet.dom.test.jsx`、`team.dom.test.jsx` |
| ctl CLI 组队链 | `ctl/tests/server_local*.rs` |

## 全量回归

共享机器（load 长期 >100）单发 `cargo test --workspace` 反复被 OOM/锁竞争杀掉，
改用直跑测试二进制电池：`cargo test --workspace --no-run` 产物按 plan 逐个执行
（341 个测试二进制、4647 条断言全绿；证据 `/tmp/oc_battery_results.txt`，含
`BATTERY COMPLETE` 标记）。四处非绿首跑均复跑转绿且与本改动无关：

| 首跑失败 | 原因 | 复跑 |
| --- | --- | --- |
| `core::tool_filter`/`tool_output_image` rc=127 | plan 引用旧 hash，二进制已被并发重编译替换 | 16/16、5/5 过 |
| `session::mcp_integration` rc=101 | 直跑缺 `CARGO_BIN_EXE_mcp_mock_server`（正常由 cargo 注入） | 注入后 7/7 过 |
| `tui::perf_long_session` 等 3 个 rc=126 | 并发编译瞬时覆盖二进制 | 全过（4/1/1） |
| `web::web_project_runs` 3 例 | load≈200 下的轮询超时 flake | 单跑 0.19s 4/4 过 |

另：`cargo build --workspace` 通过；`nodes_smoke_proc` 直跑二进制 20.44s 通过
（smoke_nodes.sh 零 team/brain 引用，与组队改动无交集）。
`cargo clippy --workspace --all-targets -- -D warnings` 通过（0 warning，6m24s）。

## 评审修正（随本迭代落盘）

- **P1** `fleet/detail/workloads.jsx` 团队执行详情成员 Tag 仍读已删字段
  `member.id/role`（渲染空 Tag + React key 告警）→ 改读 `member.agent`，
  固化的 `capabilities` 随 Tag 以 ` · ` 连带展示，captain 金标；`fleet.dom.test.jsx`
  补非空成员 fixture 与渲染断言（此前 `members: []` 未拦截）。
- **P2** `control/api/brain.rs`：绑定查找 `Err` 由静默跳过改 `tracing::warn`
  （fail-visible，避免 resolve 固化空快照无迹可查）；bind 存储与分组键统一
  `trim` 归一化（`" act"` 与 `"act"` 不再分组/匹配双输），`agents.rs` e2e 补
  padded 绑定归一断言。
- **P3** 校验错误文案指明保留对象为团队名（见上）；`role_gate` 对
  `/api/brain/agents` 的拒绝断言从仅 User 扩到循环内 User/Root；SPA roster
  「暂无能力画像」兜底文案补断言；clippy 结果补记入全量回归节。

修正后定向回归：SPA `fleet/`+`ui/`+`team.dom` 12 文件 70 例绿；`core::fleet::protocol`
4 例、`control` role_gate 3 例 + e2e `brain_api` 20 例 + `teams_dag_defs` 7 例、
`worker` team 1 例全绿。

## 兼容与范围

- Fleet 线协议版本不变（TeamDefinition 为业务负载，非 PROTOCOL_VERSION 门控字段）。
- 本地 web 链（`web/api_teams.rs`，按注册节点组队）不含职责录入，本次不动。
- `Config::default` 补齐工作区在途 DAG 配置的缺失字段（`dag: DagConfig::default()`），
  系解锁编译的最小必要修复，非本需求语义。
