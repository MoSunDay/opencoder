Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# team 模块

团队目录与消息扇出运行时；captain 是唯一规划者，成员恒 act 模式。
权威进度在 worker 本地团队目录，成员以 agent 名标识。

## 索引
- `crates/team/src/runtime.rs` — plan/sub-turn/closing 状态机
- `crates/team/src/decide.rs`、`src/prompts.rs` — 决策校验与提示
- `crates/team/src/fs_store/` — 目录 IO 与 `<team>/<topic>/` 布局
- `crates/team/tests/topic_contract_flow.rs` — 话题推进合约回归
