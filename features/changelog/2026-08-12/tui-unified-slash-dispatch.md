Commit: (working-tree, pre-initial-commit)

# TUI 统一 slash 命令分发：自由文本提交走与 `/` 弹窗相同的分发路径

## 背景

TUI 的 Submit 路径（输入框输入 `/command` + Enter）只拦截了 `/annotation`、`/notepad`、`/ps`/`/stop`/`/ap` 三类命令。其余 13 个 slash 命令（`/compact`、`/task`、`/model`、`/config`、`/mcp`、`/act`、`/plan`、`/act_clear_context`、`/install_tools`、`/fork`、`/cache_salt`）全部作为普通文本发送给 LLM，而非执行对应操作。

`command::parse()`（`command.rs:182`）能正确解析全部 16 个 `SlashAction` 变体及别名，但从未被生产代码调用。

## 变更

### 统一分发函数（tui）

- **`crates/tui/src/app_loop_actions.rs`**（106 → 250 行）：新增 `dispatch_slash_action` 函数——从 `dispatch_command` 提取全部 16 个 `SlashAction` match arm，签名去除 popup 专用参数（`command_menu`、`k`、`_keymap_menu`），使自由文本路径和弹窗路径共用同一分发逻辑。
- **`crates/tui/src/app_loop.rs`**（790 → 658 行）：`dispatch_command` 的 match 块简化为单行委托 `dispatch_slash_action(action, …)`；仅保留 `FillInput` 和 `Idle` 两个 popup 专用分支。清理由此产生的未使用 import（`SlashAction`、`local_cmd`、`ConfigForm`/`ProviderList`、`gate_compact`/`CompactGate`、`ModeSwitch`/`dispatch_mode_switch`）。

### Submit 路径接入统一分发（tui）

- **`crates/tui/src/app.rs`**（806 → 798 行）：Submit 路径将 `/annotation` + `/notepad` + `local_cmd` 手工 if-else 链替换为 `command::parse(&clean)` + `dispatch_slash_action`。未识别的输入（`parse` 返回 `None`）走原有 `push_user` + `start_turn` 路径。`is_pure_control` 分支不再需要——裸 `/act`/`/plan`/`/act_clear_context` 现在被 `parse` 拦截并通过 `dispatch_mode_switch` 的 running gate 处理。

### 清理死代码（tui）

- **`crates/tui/src/control_helpers.rs`**（62 → 48 行）：删除 `is_pure_control_cmd` 函数（`#[allow(dead_code)]` 标注的死代码，统一分发后无引用方）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| `/model` + `/mdl` 别名解析 | `parse_model_and_alias` | `command.rs` |
| slash 命令分发（全变体） | `dispatch_command` → `dispatch_slash_action` 间接覆盖 | `app_loop_dispatch_cmd_tests.rs` |
| `dispatch_slash_action` 直达路径（idle/running 门控） | `slash_action_compact_idle_starts_turn`、`slash_action_compact_running_pushes_busy_marker` | `app_loop_slash_action_tests.rs` |
| 模式切换门控 | `dispatch_mode_switch` 现有测试 | `app_loop_actions.rs` |

- 全量回归：`cargo test -p opencoder-tui --lib` → 1238 passed; 0 failed
- clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告
- 行数：`app.rs` 798 ≤ 800, `app_loop.rs` 658 ≤ 800, `app_loop_actions.rs` 250 ≤ 400

## Impact Surface

- **用户可感知**：在输入框直接输入 `/compact`、`/task`、`/model` 等 + Enter，现在正确执行对应操作（与 `/` 弹窗选择行为一致），不再泄露给 LLM 作为文本提示。
- **行为变更**：裸 `/act`/`/plan`/`/act_clear_context` 在 turn 运行时被拒（running gate，显示 busy marker），此前是直接发送给 worker。
- **不影响**：CLI/Web/session/store 边界；`/plan <content>` 等复合命令仍走原有 prompt 路径。

## Related Docs

- [agents/tui](../../agents/tui/index.md)
- [既有 `/mcp` 变更](./mcp-slash-command-and-prompt-injection.md)
