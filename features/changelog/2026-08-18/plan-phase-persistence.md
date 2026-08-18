Commit: (working-tree, post-860831d)

# plan 阶段状态落库：压缩前计划快照 + plan_input_count 持久化，resume 后 handoff 仍可武装

## 背景

plan→act handoff 依赖两份易失状态，重启/压缩后双双失效：
1. **计划文本**：`final_plan_text` 取"最后一条非空 assistant 文本"。压缩把计划折进 user 角色摘要头后它就找不到了——压缩过的 plan 会话 handoff 静默退化为纯模式切换，计划丢失。
2. **溯源门计数**：`plan_input_count`（本阶段已提交需求数）只活在内存。重启/重开 TUI 后归零，Shift+Tab 溯源门（见 [shift-tab-double-tap-fake-handoff](shift-tab-double-tap-fake-handoff.md)）永远不武装，`/act_clear_context` 永远降级。

## 变更

### session：plan 阶段生命周期模块
- **`crates/session/src/plan_phase.rs`**（新，128 行）：`reset_plan_phase`（切入 plan 模式清零计数与快照，防上阶段残留泄漏）、`persist_plan_phase`（best-effort 落库镜像）、`after_handoff`（消费快照）、`maybe_tag_plan_prompt` 计数递增后持久化。
- **`crates/session/src/compaction/mod.rs`**：压缩前捕获计划快照落库——计划 assistant 消息被折叠进摘要头前抢救出全文。
- **`crates/session/src/plan_handoff.rs`**：`final_plan_text` 为空时回退 `plan_snapshot`——压缩过的 plan 会话仍能交出计划而非静默降级。
- **`crates/session/src/resume.rs` / `control_cmd.rs` / `fork.rs`**：resume/fork 恢复 `plan_snapshot + plan_input_count`；切回 act / handoff 后清零。
- **`crates/web/src/handle.rs`**：drain 路径 handoff 后经 `SessionPatch` 清快照并回写计数。

### store：schema v10 两列 + patch 契约
- **`crates/store/src/types.rs`**：`SessionMeta` 新增 `plan_snapshot: Option<String>`、`plan_input_count: i64`；`SessionPatch` 新增 `plan_snapshot / plan_input_count / clear_plan_snapshot`，set 与 clear 互斥校验。
- **`crates/store/src/libsql_store/schema.rs` / `sessions.rs`**：v10 加列（`plan_snapshot TEXT`、`plan_input_count INTEGER NOT NULL DEFAULT 0`），读写链路贯通。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 重置清零计数与快照 | `reset_plan_phase_clears_counter_and_snapshot` | `crates/session/src/plan_phase.rs`（unit） |
| handoff 消费快照 | `after_handoff_consumes_snapshot` | 同上 |
| 落库镜像往返 | `persist_plan_phase_round_trip` | 同上 |
| 压缩捕获快照 + handoff 回退找回 | `compaction_snapshots_plan_and_handoff_recovers` | `crates/session/tests/plan_snapshot_compaction.rs` |
| 无计划的二次压缩保留既有快照 | `second_compaction_without_plan_keeps_existing_snapshot` | 同上 |
| act 模式压缩永不捕获 | `act_mode_compaction_never_captures_snapshot` | 同上 |
| resume 重新武装溯源门 | `resume_restores_plan_phase_arming` | 同上 |
| patch 往返 / set-clear 互斥 / 默认值 | `plan_snapshot_round_trip_via_patch` 等 3 项 | `crates/store/tests/plan_phase.rs` |

- 全量回归：用户豁免当次复跑（已测）；同工作树验证记录见 [skill-full-body-injection](skill-full-body-injection.md)：`cargo test --workspace` 2943 passed / 0 failed，clippy 零警告。
- 连锁适配：约 30 个测试文件的 `SessionMeta` 字面量补两个新字段（+2 行/文件），无行为变化。
- 行数：新文件 117–280 行 ≤400。

## Impact Surface
- plan 会话跨压缩/重启后 handoff 不再丢计划、溯源门可重新武装；act 会话行为不变。
- store schema v9→v10 自动迁移，旧库升级后旧 plan 会话无快照（首次压缩起开始捕获）。
- 不影响：CLI/Web API 形状、消息与事件 schema。

## Related Docs
- [agents/session](../../agents/session/index.md)
- [agents/store](../../agents/store/index.md)
- [shift-tab-double-tap-fake-handoff](shift-tab-double-tap-fake-handoff.md)（溯源门 UI 侧修复）
