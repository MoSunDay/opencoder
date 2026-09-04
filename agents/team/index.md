Commit: (working-tree, team 功能并行迭代中，随收口会话统一落盘)

# team 模块

## 职责

`opencoder-team` 是多 agent 组队讨论运行时：一个**团队** = 队长 node + 若干组员 node（全部是注册的 fleet node，见 [agents/node](../node/index.md)），围绕一个**话题**多轮讨论对齐。执行面完全复用既有 node_tasks 派发链路——每次提问就是一条合成 node 任务，节点用本地配置与 LLM 凭证跑完整 agent session，运行时只取末条 assistant 文本；server/web 只做控制面（建队、发起、取消、恢复、观测）。全部权威执行状态放共享目录（生产环境 NFS 挂载，`Config.team_root`）：磁盘布局即进度，游标每步纯推导，`run_topic` 崩溃后重跑即 resume。

一轮（turn）的结构由三个队长 JSON 决策驱动：

1. **plan（决策 ①）**：队长结合历史轮摘要与成员能力画像，给出本轮问题与参与成员，写 `plan.json`。
2. **sub-turns**：sub 0 各成员作答；决策 ② 在每个 sub-turn 后总结成员结果并判定对齐——不对齐则点名存疑成员做歧义追答（sub ≥ 1，上限 `max_sub_turns`）。
3. **closing（决策 ③）**：对齐后收束话题（complete + 最终总结）或带 hint 开下一轮（上限 `max_turns`）。

## 边界

- 不实现 HTTP（路由在 [agents/web](../web/index.md)），不实现节点执行（[agents/node](../node/index.md)）；本 crate 只依赖 `Arc<dyn Store>` 与 `TeamDispatcher` 两个抽象。
- 共享目录是唯一权威执行状态；DB 的 `team_topic_runs` 只是 `(topic, node)` 台账（观测与终态收束用），不承载进度——丢库不丢讨论。
- 队长回复必须是结构化 JSON：解析/校验失败带错误反馈重问（`decide.rs::PARSE_RETRIES=2`，共 3 次询问；传输失败重试 1 次），仍失败按成员失败容忍或话题 `error` 收束，绝不猜测半截输出。
- 共享目录所有文件有尺寸上下界（`fs_store.rs`：2B..=2MiB）——失控模型回复写不爆 share。
- 共享目录未实现 NFS server 本身：只约定「可被多机同时读、原子写可见」的目录语义。

## 模块地图

- `layout.rs`：共享目录路径构造 + 名字校验（纯函数）。团队名 `^[a-z0-9][a-z0-9-]{0,63}$`、topic_id 必须是 ULID、turn 1..=999 / sub 0..=999、成员 id `[A-Za-z0-9_-]{1,64}`——路径段先校验后拼接，恶意名字无法穿越 `team_root`；另有两个目录扫描 `list_team_dirs` / `list_topic_dirs`（坏名字直接跳过）。
- `types.rs`：元信息 DTO（`TeamMeta` / `TopicMeta` / `PlanRecord` / `ResultRecord` / `SummaryRecord` 与三个决策形状）+ 状态常量；`parse_decision` 容错解析（裸 JSON / 单个完整 `` ```json `` 围栏（允许周围有散文）/ 散文夹单个对象，双围栏或截断围栏判错让运行时重问）。
- `fs_store.rs`：全 crate 唯一碰共享目录 IO 的文件。写一律 tmp（`.<name>.tmp-<ulid>`）→ rename 原子替换，读者只见旧文件或新文件；读带尺寸界与损坏容忍（`read_topic_tree` 聚合整棵话题树，坏文件跳过告警）。
- `prompts.rs`：七个提示词纯函数（选人 plan / 作答 answer / 对齐追答 alignment / 总结 summary / 收尾 closing / 能力画像 profile / 纠错 correction）+ `truncate`。
- `decide.rs`：队长决策管线的共享件——`ask_json`（解析+校验+纠错重问）、`ask_with_retry`（传输重试）、四个 validator、成员提示/结果记录构造、`capability_table`（选人时注入能力画像）、`turn_digests`（历史轮摘要注入）。
- `dispatcher.rs`：`TeamDispatcher` trait + `NodeDispatcher`（生产）+ `MockDispatcher`（脚本化测试）。
- `cursor.rs`：`Cursor { turn, stage }` 纯磁盘推导。
- `runtime.rs` + `runtime/stages.rs`：`start_topic` / `run_topic` 状态机（plan→sub→closing 三 stage 函数）。
- `terminal.rs`：唯一终态迁移 `finish`。
- `profile.rs`：`profile_team` 成员能力画像——逐成员窄合并（成功访谈后重读 team.json 只回写该成员 capabilities/profiled_at），并发改队长/成员管理不被旧快照回滚。
- `config.rs`：`TeamRunConfig`（从 `Config` 收窄出三个旋钮）+ `CancelToken`（协作式取消，步间显式检查，无异步取消魔法）。

## 关键抽象

- `TeamDispatcher`（`dispatcher.rs`）：`ask(topic, node_id, prompt) -> 末条 assistant 文本`。`topic == None` 表示非话题调用（能力画像），不写台账。
  - `NodeDispatcher`（生产）：`dispatch_node_task` 建合成任务（agent=None）→ `upsert_team_topic_run` 记台账 → 轮询至终态（默认 2s 间隔 / 30min 超时；超时对节点发 cancel 请求并以 error 返回）；done 后取该 session 末条 assistant 文本。
  - `MockDispatcher`：按节点 FIFO 队列脚本回放（`mock.reply("n1", vec![ok(json), err("boom")])`），可选携带 store 让台账 upsert 真实发生——零 token 驱动全部 runtime 测试。
- 磁盘游标（`cursor.rs`）：每步从布局重新推导执行位置——无 plan 的 turn → `Plan`；turn 已记入 `meta.turns` → `Closing`；否则**最大已存 summary 是权威**：若 `aligned` 则 turn 已定，走 `Record`（读盘补记 `meta.turns`）直接进 Closing，绝不派发幻影追答轮（覆盖 summary 落盘与 turns 入账之间崩溃的窗口）；未对齐时「最小无 summary 的 sub-turn」是工作前沿（其缺失成员 result 重新派发；全部有 summary 时重评估最大 sub-turn）。因此 **resume = 再跑一次 `run_topic`**，无任何内存状态需要恢复。
- `run_topic`（`runtime.rs`）：主循环「查 cancel → 推导游标（或消费 pending_plan）→ 执行 stage → 终态则返回」。closing 判 `continue` 时设置 `pending_plan` 强制下一轮走 Plan（此间崩溃只是重问 closing，安全）。
- `TeamHub`（web 侧，`crates/web/src/team_hub.rs`）：进程内「活着的 topic 运行时」注册表——每话题仅一个 `CancelToken` 句柄；条目存在 = 运行时 task 存活，**不是**话题未完成（server 重启后 executing 话题无条目，正是 resume 候选）。register 时先取消并替换遗留条目，杜绝陈旧 token。

## 共享目录布局

```text
<team_root>/<team_name>/team.json                          ← 团队元信息（队长/成员/能力画像）
<team_root>/<team_name>/<topic_id>/team.json               ← 话题元信息（状态机 + 轮次账本）
<team_root>/<team_name>/<topic_id>/<turn>/plan.json        ← 决策 ①（本轮问题/参与成员/理由）
<team_root>/<team_name>/<topic_id>/<turn>/<sub_turn>/<member>/result.json   ← 成员作答/追答/失败
<team_root>/<team_name>/<topic_id>/<turn>/<sub_turn>/summary.json           ← 决策 ②（总结/对齐判定/歧义点名）
```

turn 从 1 计、sub_turn 从 0 计。话题元信息固定叫 `team.json`（按用户指定的文件名）；`TopicMeta.turns` 是已完成轮的账本（对齐戳 + 首末 sub-turn），终态字段 `finish_reason`/`finished_at`/`final_summary` 只在 finish 时写。

## 话题状态机

`executing → finished`，`finish_reason ∈ complete | max_turns | max_sub_turns | cancelled | error`：

- `complete`：closing 判对齐收束（带最终总结）；`max_turns` / `max_sub_turns`：轮数上限收束（防永不收敛）；`cancelled`：CancelToken 触发或 web 无运行时路径直接落盘；`error`：队长决策不可用（resumable）。
- `terminal::finish` 一次逻辑步内先写话题终态元信息、再翻 `team_topic_runs` 该 topic 全部行为 `finished`——两写各自幂等，中间崩溃重试收敛；「磁盘已终态 + 台账仍 executing」残态在生产 HTTP 面的唯一收敛入口是 resume 的 409 拒绝分支（`api_teams_topics.rs` 先幂等补翻再拒绝）。
- 唯一可逆终态是 `error`（resume 把状态翻回 executing）；成员个体失败不终止话题，落 `result.json {ok:false}` 由决策 ② 容忍。

## 与 node / todos 范式的关系

- 对节点的唯一契约就是既有 node_tasks 派发链路（claim→执行→终态上报）：node crate 零改动，天然继承「节点本地配置与凭证、断流续传、lost 收束」。
- 与 [todos](../todos/index.md) 同为「父编排 + 子执行」范式，但权威状态分置不同：todos 全在 Store（generation CAS），team 在共享目录（磁盘游标）；web 侧都走「hub + tokio::spawn + 可注入抽象」的测试接缝（todos 是 client_override，team 是 dispatcher）。

## 依赖与接口

- 依赖 `opencoder-core`（Config 三旋钮、`now_ms`）、`opencoder-store`（node 注册表校验 + `team_topic_runs` 台账三方法）。
- 被 `opencoder-web` 依赖（路由与 TeamWebState，见 [agents/web](../web/index.md)）。
- 配置：`Config.team_root`（默认 `<data_root>/team`，env `OPENCODER_TEAM_ROOT`；web 启动时未显式设置的根会重定到当前 workdir 数据目录 `<data_dir>/team`，与库文件同树便于备份清理）、`team_max_turns`（默认 8）、`team_max_sub_turns`（默认 3）。

## 代表性验证

- `crates/team/tests/layout_types.rs`（11）：名字/ULID/上下界校验、路径构造先校验拒绝穿越、坏名字目录列举跳过、`parse_decision` 三形态接受与垃圾拒绝、字段形状、fs_store 团队生命周期与坏文件跳过、原子写尺寸界与话题树往返。
- `crates/team/tests/dispatcher_flow.rs`（3）：真 `NodeDispatcher`+真 LibsqlStore——done 路径（claim→作答→complete→末条 assistant 文本+台账 upsert）、error 任务透传、超时发 cancel。
- `crates/team/tests/runtime_flow.rs`（9）：一次对齐完整跑通并断言全盘布局、歧义点名走第二轮追答、持续不对齐 `max_sub_turns` 收束、成员失败容忍并记录、队长 JSON 纠错带反馈重问、永不收敛 `max_turns` 收束、error 话题 resume 不重放已完成工作、aligned-summary 崩溃残态直接入账不派发幻影追答轮、终态话题重试补翻 executing 台账。
- `crates/team/tests/termination_flow.rs`（3）：cancel 终态+台账翻转、`profile_team` 扇出并落能力画像、画像窄合并在并发加成员后不回滚管理操作。
- store 面：`crates/store/tests/team_runs.rs`（upsert 冻结 created_at / finish 全行翻转 / 节点删除级联）与 `store_migrations.rs` 的 v16→v17 迁移；web 面：`crates/web/tests/api_teams.rs`（10，含 resume 409 拒绝路径收敛 executing 台账残态）+ SPA `teamItems.test.js` / `team.dom.test.jsx`。
