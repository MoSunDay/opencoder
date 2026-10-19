Commit: 9a8213e3dba80353fefcf7d20aa57bf7461f27c8

# 返工后继续前进的反思字段

历史 `run.reflection` 可能被模型复制到前向派发，导致连续纠错仍被拒绝。提示词现在明确：只有返工或本层重试填写反思，前进时 `reflection` 必须为 null；校验错误同时给出具体修改方式。历史反思、层屏障、评估和轮次规则保持不变。

功能 → 测试：`forward_after_rework_rejects_copied_reflection_and_accepts_null_without_another_round` 覆盖真实状态机中的测试失败、返回修复、拒绝携带旧反思的前进，以及纠正后在同一轮继续下一层。

验证范围：`cargo test -p opencoder-brain --test milestone`，20 项通过；`cargo clippy -p opencoder-brain --all-targets -- -D warnings` 和该 crate 的格式检查通过。发布状态及真实模型运行记录保存在仓库外的交付证据中。
