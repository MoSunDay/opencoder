# clear-context / plan→act handoff 未持久化清除 skill → resume 重载过期 skill

## 背景

`/act_clear_context`（`ClearContext` 控制命令）的 `apply()` 调
`session.set_skill(None)` 清掉了**内存中的** `skill_prompt`，但
`persist_clear()` 写回 store 的 `SessionPatch` 只带 `clear_summary`，没设
`clear_skill`。store 的 `skill` 列因此保持原值（`None=跳过` 语义无法表达
「置 NULL」）。

而 `resume.rs` 在重建 `SessionState` 时从 `meta.skill` 还原 skill：

```
skill_prompt: Arc::new(Mutex::new(meta.skill.clone())),
active_skill_names: Arc::new(Mutex::new(infer_skill_names(&meta.skill))),
```

结果：清空上下文（或 plan→act 交接）后一旦 resume，会重新加载那条已清掉的
skill，污染系统提示并可能重新解锁本应失效的工具。Web API 路径
（`api_ops.rs`）已于 2026-08-04 用 `clear_skill: true` 修复，但 session
（`control_cmd` / `autopilot`）与 TUI（worker `SwitchAndStart`）路径未对齐。

## 变更

三个 handoff/clear 持久化点补 `clear_skill: true`，使 store 的 `skill` 列
与内存同步置空：

- **`crates/session/src/control_cmd.rs::persist_clear`**：`/act_clear_context`
  路径。`SessionPatch` 增加 `clear_skill: true`（与既有 `clear_summary: true`
  并列）。
- **`crates/session/src/autopilot/phases.rs::run_act_phase`**：autopilot
  plan→act handoff 路径，同补 `clear_skill: true`。
- **`crates/tui/src/worker.rs`**：TUI `SwitchAndStart`（Shift+Tab /
  `/act_clear_context` from plan）持久化点，同补 `clear_skill: true`。

纯追加已存在的 `bool` 标志，store 层 `clear_skill` 原语自 v6 支持，无 schema
变更。

## 兼容性

- `SessionPatch::clear_skill` 默认 `false`，`#[serde(default)]` 兼容旧客户端。
- 三处改动使 session/tui 路径对齐 web API 既有模式，不影响其它字段语义。

## 测试覆盖

| 断言 | 测试名 | 文件 |
|------|--------|------|
| ClearContext 后 store.skill == None（in-memory 也 None） | `apply_clear_context_clears_skill_in_store` | `session/src/control_cmd.rs`（单元，新增） |
| 无 skill 时 ClearContext 为 no-op、不 panic | `apply_clear_context_with_no_skill_is_harmless` | `session/src/control_cmd.rs`（单元，新增） |
| `clear_skill:true` NULL 化 skill 列、无关字段不动 | `clear_skill_nulls_skill_field` | `store/tests/clear_skill.rs`（既有） |
| 默认 patch 不触碰 skill 字段 | `default_patch_leaves_skill_intact` | `store/tests/clear_skill.rs`（既有） |
| TUI SwitchAndStart 后 skill_prompt 为 None | `switch_and_start_clears_skill_prompt` | `tui/tests/plan_act_handoff.rs`（既有） |

## Gate

| 项 | 结果 |
|----|------|
| `cargo test --workspace` | 1892 passed; 0 failed; 0 ignored |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace --tests` | Finished，零错误零警告 |

行数约束：`control_cmd.rs` 553 / `phases.rs` 98 / `worker.rs` 413，均 <800。
