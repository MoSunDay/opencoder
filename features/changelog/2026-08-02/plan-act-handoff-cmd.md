Commit: (working-tree, pre-initial-commit)

# `/act` 与 `/act_clear_context` 在 plan 模式下走 plan→act 交接

## 背景
在 plan 模式下提交计划后，用户期望 `/act`（和 `/act_clear_context`）的行为与
`Shift+Tab` 一致——**保留计划、切换到 act 模式并自动开始执行**。但此前这两个 slash
命令无条件 dispatch 控制命令 prompt（`/act`、`/act_clear_context`），经
`apply_control_with_registry` 短路后会**清空整个 transcript**——计划丢失。

## 变更

### plan→act 交接路由
- **`crates/tui/src/app_loop.rs`**：`dispatch_command` 对 `SlashAction::Act` 和
  `SlashAction::ClearContext` 新增前置判断——当 `chat.agent == "plan" &&
  chat.plan_submitted && !*running` 时，走 `SwitchAndStart("act", extra)`（与
  `Shift+Tab` 同路径），而非 dispatch 控制命令 prompt。
  新增 `prep_plan_to_act` helper：清空 input、刷新 sys_tokens 基线、设置
  mode_flash 横幅，返回捕获的 input 文本作为 `extra` payload。
- **`crates/tui/src/app.rs`**：`dispatch_command` 签名新增 `mode_flash`、`anim_tick`、
  `sys_tokens` 参数（供 `prep_plan_to_act` 使用）；调用点同步更新。

### 测试
- **`crates/tui/src/app_loop_dispatch_cmd_tests.rs`**（新增 333 行）：5 条测试覆盖
  plan→act 交接与 fallback dispatch 的全部分支。
- **`crates/tui/src/app_loop_tests/mod.rs`**：注册新测试模块 `dispatch_cmd_tests`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| `/act` 从 plan+已提交计划 → SwitchAndStart 交接 | `slash_act_from_plan_with_plan_routes_handoff` | `app_loop_dispatch_cmd_tests.rs` |
| `/act_clear_context` 从 plan+已提交计划 → 交接 | `slash_clear_context_from_plan_with_plan_routes_handoff` | 同上 |
| `/act_clear_context` 从 act 模式 → 仍 dispatch 控制命令 | `slash_clear_context_from_act_mode_dispatches_prompt` | 同上 |
| `/act` 从 act 模式 → 仍 dispatch 控制命令 | `slash_act_from_act_mode_dispatches_prompt` | 同上 |
| `/act` 从 plan 但无计划 → 仍 dispatch 控制命令 | `slash_act_from_plan_without_plan_dispatches_prompt` | 同上 |

- 全量回归：`cargo test --workspace` → 1642 passed / 0 failed / 1 ignored
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：`app_loop_dispatch_cmd_tests.rs` 333（新增 ≤400）；`app_loop.rs` 781（迭代 ≤800）

## Impact Surface
- TUI 用户：plan 模式下 `/act`、`/act_clear_context` 不再清空计划，改为自动执行（同 Shift+Tab）。
- 不影响：act 模式下的 `/act`、`/act_clear_context`（仍为控制命令）；session runner / store 数据形状。

## Related Docs
- [既有 plan→act 手动切换](../2026-07-06/plan-act-handoff-compact.md)
