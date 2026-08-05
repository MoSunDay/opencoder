Commit: (working-tree, pre-initial-commit)

# steer `>` 一键取消子智能体并提交（不再需要点两次）

## 背景
父会话正在运行且有活跃子智能体时，用户先按 Enter 提交 steer（持久化入队），再点击 steer 行的 `>` 按钮意图立即打断并吸收。但 `>` 只硬取消了子智能体（`CancelChildren`），没有触发父级 `turn_cancel`，父级继续当前 turn 直到自然边界才吸收 steer。用户体感为「打断了子但没提交」，需要再点一次 `>`（此时子已消失，resolve 回落 `SteerParent`）才能触发中断。预期一次点击同时取消子 + 引导父级。

## 变更

### steer_dispatch: 新增 `CancelChildrenAndSteer` 动作
- **`crates/tui/src/steer_dispatch.rs`**（`Action` 枚举, :20-23）：新增 `CancelChildrenAndSteer` 变体，表示「子已被取消 + 父级 turn 也需立即中断」。
- **`crates/tui/src/steer_dispatch.rs`**（`resolve`, :47-53）：当 `running && has_children` 时区分 `has_pending_steer`——有 steer 返回 `CancelChildrenAndSteer`，无 steer 保持原 `CancelChildren`（仅取消子，等待自然边界）。

### steer_fire: `CancelChildrenAndSteer` 触发 turn_cancel
- **`crates/tui/src/steer_fire.rs`**（`fire_steer_interrupt` match, :95-97）：将 `CancelChildrenAndSteer` 与 `SteerParent` 合并到同一 arm，都调用 `fire_turn_cancel`。子取消已在 `resolve` 之前的 `fire_child_cancels` 完成，此处只补上父级中断。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| 有子+steer → CancelChildrenAndSteer | running_parent_with_children_and_steer_steers_parent_too | crates/tui/src/steer_dispatch.rs |
| 无子+steer → CancelChildren | running_parent_with_children_cancels_children | crates/tui/src/steer_dispatch.rs |
| 有子+steer → fire turn_cancel | running_parent_with_children_and_steer_fires_turn_cancel | crates/tui/src/steer_fire.rs |

- 全量回归：`cargo test --workspace` → 1916 passed, 0 failed, 1 ignored
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 0 warnings
- 行数：steer_dispatch.rs 123 ≤ 800；steer_fire.rs 407 ≤ 800

## Impact Surface
- TUI：steer `>` 按钮在有子运行时一次点击即可取消子 + 立即吸收 steer（用户可感知）。
- 不影响：Store/LLM/Web/CLI 边界；键盘 Enter 提交路径（`admit_keyboard_steer`）行为不变。
- session runner 无需改动——`fire_child_cancels` + `fire_turn_cancel` 操作不同 mutex，biased select! 保证安全中断。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
- [既有相关 changelog](../2026-08-05/tui-steer-enter-no-interrupt.md)
