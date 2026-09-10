Commit: (working-tree, 基于 b465f440)

# team 模块

可嵌入多 agent 讨论状态机，不决定节点调度。

## 关键路径

- `crates/team/src/dispatcher.rs` — TeamDispatcher::ask 接缝；旧 NodeDispatcher 兼容
- `crates/team/src/runtime.rs` + `runtime/stages.rs` — plan/sub-turn/closing 状态机
- `crates/team/src/cursor.rs` — 从持久化计划/结果/summary 推导恢复位置
- `crates/team/src/decide.rs` — 校验队长 JSON 决策
- `crates/team/src/prompts.rs` — 计划/回答/对齐/总结提示
- `crates/team/src/layout.rs` — 团队名与 topic 校验（ULID、team-/system-）
- `crates/team/src/fs_store/` — 目录 IO；rename 原子替换、读取限尺寸
- `crates/team/src/terminal.rs` — 统一写终态
- `crates/team/src/profile.rs` — 成员能力画像（best-effort）
- 目录形状 `<team_root>/<team>/<topic>/team.json` 及逐轮 plan/result/summary

## 边界

- system 团队执行已退役：control 与 worker 均拒绝。
- 权威进度在 worker 本地团队目录；`team_topic_runs` 仅运行台账。
- 成员以 agent 名标识（`MemberRef.node_id = name = agent`，团队内唯一）；不经 NFS 共享话题内容。
- 成员 `capabilities` 是控制面 resolve 固化的大脑能力 summary 快照（非运行时画像）。

## 相关

- [control](../control/index.md) 团队定义与选点；[worker](../worker/index.md) 执行
- [Agent 平台](../../features/agent-platform/index.md)
