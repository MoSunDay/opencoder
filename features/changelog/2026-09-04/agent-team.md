Commit: (working-tree, team 功能与 daemon→server 拆分等并行迭代共享工作树，随收口会话统一落盘)

# opencoder-team：多 agent 组队讨论——队长/组员 node 多轮对齐 + 共享目录权威状态 + Web 组队/话题面板

## Context

用户需求：多个 opencode-agent（fleet node）组成团队，由**队长 node** 带领**组员 node** 围绕一个话题多轮讨论对齐；队长需维护组员**能力画像**用于选人；生产环境各 node 分布多机，讨论的权威状态放**共享 NFS 目录**；服务端（server）需落库记录话题的**创建时间 / node_id / topic_id / 状态**供观测与收束。

本轮新增 `opencoder-team` crate + store schema v17 一张台账表 + web `/api/teams*` 路由组 + SPA「组队/话题」面板，形成建队→画像→发起话题→多轮讨论→收束/取消/恢复的闭环。执行面零新协议：完全复用既有 node_tasks 派发链路（server 只做控制面编排），node crate 零改动。

## 方案要点（关键决策）

- **D1 执行面复用 node_tasks**：队长/组员的每次提问=一条合成 node 任务（agent=None），节点用本地配置与 LLM 凭证跑完整 agent session，运行时轮询终态取末条 assistant 文本——天然继承断流续传、lost 收束，无新节点协议。
- **D2 权威状态在共享目录，DB 只做台账**：`<team_root>/<team>/<topic>/...` 树承载全部执行状态（元信息/plan/成员 result/summary），tmp+rename 原子写；执行位置（游标）每步从磁盘**纯推导**——崩溃后 resume 就是再跑一次 `run_topic`，无检查点、无内存状态。DB 的 `team_topic_runs` 只记 `(topic_id, node_id, status, created_at)` 配对台账（用户要求的服务端记录），不承载进度。
- **D3 share 安全三件套**：路径段先校验后拼接（团队名 `^[a-z0-9][a-z0-9-]{0,63}$`、topic_id 必须 ULID、turn/sub 三位数封顶、成员 id 同 node id 规则）防穿越；单文件尺寸界 2B..=2MiB 防失控回复撑爆 share；读路径损坏容忍（坏文件跳过告警，半写树不 break 列表）。
- **D4 `TeamDispatcher` 接缝**：`ask(topic, node, prompt) -> 末条 assistant 文本` 一个 trait——生产 `NodeDispatcher`（派发+轮询+台账 upsert+超时 cancel）、测试 `MockDispatcher`（按节点 FIFO 脚本回放），web 层 `TeamWebState` 可注入，全部 runtime/web 测试零 token。
- **D5 队长结构化 JSON 决策 + 容错**：plan/summary/closing 三决策必须 JSON；`parse_decision` 接受裸 JSON/单个完整 ```json 围栏（周围可有散文）/散文夹单对象，双围栏或截断判错；解析或校验失败带错误反馈纠错重问（`PARSE_RETRIES=2`，共 3 次）；成员个体失败不终止话题（落 `result.json {ok:false}` 由 summary 决策容忍），队长决策彻底失败才 `finished(error)`（可 resume）。
- **D6 有界收束**：外层 `max_turns`（默认 8）/内层 `max_sub_turns`（默认 3）双上限防永不收敛；`CancelToken` 步间协作取消；web cancel 幂等双路径（活运行时走 token，重启遗留/error 话题直接落盘 `finished(cancelled)` + 台账翻转）。

## 落地清单

- **core**：`Config.team_root`（默认 `<data_root>/team`，env `OPENCODER_TEAM_ROOT`）、`team_max_turns`（8）、`team_max_sub_turns`（3），env `OPENCODER_TEAM_MAX_TURNS`/`OPENCODER_TEAM_MAX_SUB_TURNS`（非法值 warn 忽略）。
- **store（schema v16→v17）**：`team_topic_runs(topic_id, node_id FK→nodes CASCADE, status executing|finished, created_at, PK(topic_id,node_id))` + topic 索引；`Store` trait 增 `upsert_team_topic_run`（冲突臂只刷 status，**created_at 首插冻结**）/`finish_team_topic_run`（topic 全行翻 finished，幂等）/`list_team_topic_runs`（`created_at, rowid` 定序——ULID 非单调不可排序）三方法（默认 bail，libsql 完整实现）；新文件 `team_types.rs`、`libsql_store/team_runs.rs`（DDL 由域模块持有、schema.rs 注册进 bootstrap 批与 v17 迁移）。
- **新 crate `crates/team`**（纯函数式，13 文件）：`layout.rs`（校验+路径构造）、`types.rs`（元信息 DTO+容错 JSON 解析）、`fs_store.rs`（全 crate 唯一磁盘 IO：tmp+rename/尺寸界/坏文件容忍/整树读取）、`prompts.rs`（plan/answer/alignment/summary/closing/profile/correction 七提示词）、`decide.rs`（ask_json 纠错管线+validator+能力表/轮摘要注入）、`dispatcher.rs`（trait+Node/Mock）、`cursor.rs`（磁盘游标）、`runtime.rs`+`runtime/stages.rs`（`start_topic` 校验成员注册→落 executing 元信息；`run_topic` 状态机 plan→sub_turn 作答/歧义追答→summary→closing）、`terminal.rs`（唯一终态迁移：先写终态元信息再翻台账，两写幂等）、`profile.rs`（`profile_team` 逐成员画像，失败仅 warn）、`config.rs`（`TeamRunConfig`+`CancelToken`）。
- **共享目录布局**：`<team_root>/<team>/team.json`（团队元信息+成员能力画像）、`<team>/<topic_id>/team.json`（话题元信息，文件名按用户指定；状态机+轮次账本+final_summary）、`<topic>/<turn>/plan.json`、`<turn>/<sub>/<member>/result.json`、`<sub>/summary.json`。
- **web**：新文件 `team_state.rs`（`TeamWebState`：run cfg+可注入 dispatcher+hub；production 启动加载 Config 失败降级默认、未显式配置的 team_root 重定到 workdir `data_dir` 下、启动建根目录）、`team_hub.rs`（话题运行时注册表+`spawn_topic_runtime`；register 先取消遗留 token）、`api_teams.rs`（团队半区）、`api_teams_topics.rs`（话题半区）；`AppState.team` 挂载。路由：`GET/POST /api/teams`、`PATCH /api/teams/:name`（改队长）、`POST /api/teams/:name/members`（增删，队长不可移出）、`POST /api/teams/:name/profile`（202 后台画像）、`GET/POST /api/teams/:name/topics`（创建 201+spawn）、`GET /api/teams/:name/topics/:tid`（整棵讨论树）、`POST .../cancel`（幂等双路径）、`POST .../resume`（202；executing 孤儿/finished(error) 可续，其余或已在跑 409）、`GET /api/topics?team=`；全部自动受 HMAC 签名保护。
- **SPA**：二级导航新增「组队」「话题」tab；`teamPanel`（团队表 3s 轮询+建队/改队长/成员管理/发起话题/能力画像四模态）、`topicsPanel`（话题表+团队过滤（组队页「查看话题」预置）+取消/恢复）、`topicDetail`（左 turn Timeline（绿点=对齐）、右 plan/逐 sub-turn 成员 result 折叠/summary/歧义链/最终总结；executing 保持 3s 轮询）；`teamItems.js` 纯映射（状态/收尾原因/能力摘要/时间线/动作可用性）+测试。话题状态机：`executing → finished(complete|max_turns|max_sub_turns|cancelled|error)`，error 可 resume。

## Tests

| 层 | 覆盖 |
| --- | --- |
| core | `tests/config_contract.rs`：`defaults_when_no_config_present` 增 team 三默认断言；新增 `team_config_file_merge_and_env_overrides`（文件合并+env 覆盖）。`cargo test -p opencoder-core` 全绿（378 用例） |
| store | `tests/team_runs.rs` 3 例：upsert 往返且 **created_at 冻结**、finish 全行翻转、节点删除级联；`tests/store_migrations.rs` 增 v16→v17（手写 v16 库迁移后建表可写、version=17）。`cargo test -p opencoder-store` 全绿（186 用例） |
| team | `cargo test -p opencoder-team` 23 用例全绿：`layout_types.rs` 11（名字/ULID/上下界校验、路径构造拒绝穿越、坏名字目录跳过、`parse_decision` 三形态+垃圾拒绝+字段形状、fs_store 生命周期/坏文件跳过/原子写尺寸界/整树往返）；`dispatcher_flow.rs` 3（真 NodeDispatcher+真库：done 取末条 assistant 文本+台账、error 透传、超时发 cancel）；`runtime_flow.rs` 7（一次对齐跑通+全盘布局断言、歧义两轮追答、max_sub_turns 收束、成员失败容忍、队长 JSON 纠错重问、max_turns 收束、error 话题 resume 不重放已完成工作）；`termination_flow.rs` 2（cancel 终态+台账翻转、profile 扇出落盘） |
| web | `tests/api_teams.rs` 9 例：签名中间件 401、建队/重名 409/未注册节点 400、改队长+成员管理（含队长移出守卫）、话题全链路跑通+磁盘布局断言、cancel 双路径（token/落盘）幂等、resume 202 与终态 409、`/api/topics?team=` 过滤（含 `..%2F` 穿越 400）、profile 202+能力落盘。`cargo test -p opencoder-web` 全绿（52 suites / 260 用例） |
| SPA | `teamItems.test.js` 16（状态/收尾原因映射、能力摘要阶梯、选项构造、turn 时间线双形状、result/歧义文案、cancel/resume 可用性）+ `team.dom.test.jsx` 9（teamPanel 行为/建队模态/查看话题预过滤/画像分发、topicsPanel 渲染/过滤/取消/恢复、topicDetail 时间线/返回）。`npm test` 298/298 全绿（28 文件）；`scripts/check-spa-drift.sh` 无漂移 |

### 回归（规则 02）

- 定向：`cargo test -p opencoder-core -p opencoder-store -p opencoder-team` 与 `cargo test -p opencoder-web` 全绿（core 378 / store 186 / team 23 / web 52 suites 260）；SPA `npm test` 298/298、`check-spa-drift.sh` 通过。
- 全量 `cargo test --workspace --no-fail-fast --exclude opencode-agents`：**0 失败**（最终 gate；期间一次高负载快照下 19 个无关套件超时失败，系并行会话 129 进程把机器打到 load 120 所致，负载回落后同命令全绿）。`opencode-agents` 为并行会话在途新 crate（其测试二进制曾在高负载下挂起 38min），不属于本功能改动面。
- `cargo clippy -p opencoder-core -p opencoder-store -p opencoder-team -p opencoder-web --all-targets`：0 warning。
- `daemon_smoke` / `nodes_smoke_proc`：单独复跑通过（0.13s / 10.41s），证实早期失败为资源饥饿而非本功能（server 二进制带 `AppState.team` 正常启动并打印 listening）。
