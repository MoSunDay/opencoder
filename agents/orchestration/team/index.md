Commit: 3cc5011a805b0b3f8110623a5c4f0be40267285c

# team 模块

团队目录与消息扇出运行时；captain 是唯一规划者，成员仅产出 answer/alignment 应答，不参与规划。
权威进度在 worker 本地团队目录，成员以 agent 名标识。

## 索引
- `crates/team/src/runtime.rs` — plan/sub-turn/closing 状态机
- `crates/team/src/decide.rs`、`src/prompts.rs` — 决策校验与提示
- `crates/team/src/fs_store/` — 目录 IO 与 `<team>/<topic>/` 布局
- `crates/team/tests/topic_contract_flow.rs` — 话题推进合约回归
