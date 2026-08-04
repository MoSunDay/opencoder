Commit: (working-tree, pre-initial-commit)

# feat(tui): 斜杠命令弹窗 Tab 填充输入框时追加尾随空格

## 背景

在斜杠命令弹出菜单中按 Tab 选中某命令时，`FillInput` 仅将命令名（如 `/plan`）
填入输入框，光标紧贴命令名末尾。用户若要追加参数，需先手动按空格；若直接按
Enter，`command::parse()`（command.rs）会 `trim()`，分发正常但缺少「可直接输入
参数」的提示感。

## 变更

### app_loop.rs — FillInput 消费点追加尾随空格

在 `dispatch_command` 的 `CommandOutcome::FillInput` 分支（消费点）新增一行
`input.push(' ')`，使输入框变为 `/plan `，光标 `cursor_idx = input.len()` 落在
空格之后：

```rust
CommandOutcome::FillInput(s) => {
    input.clear();
    input.push_str(&s);
    input.push(' ');  // trailing space so args/Enter work immediately
    *cursor_idx = input.len();
    return LoopFlow::Redraw;
}
```

**为何改消费点而非构建点（command.rs 的 `FillInput` 载荷）？**
- `FillInput` 载荷保持纯粹的命令名，与 command.rs:73 文档注释一致。
- 尾随空格属于编辑器 UX 问题，归属于负责编辑器改写的 `app_loop`。
- command.rs 现有单元测试（`tab_fills_input_with_command_name` 等 4 个）载荷
  不变、保持通过。
- `command::parse()` 自动 trim，故「Tab 后直接 Enter」分发路径不受影响。

## 用户可感知变化

- `/` 弹窗中 Tab 选中命令后，输入框为 `/plan `（光标在空格后）。
- 可立即输入参数，或直接按 Enter 分发（parse 自动修剪尾随空格）。

## 测试覆盖

| 闸门 | 结果 |
|------|------|
| `cargo test -p opencoder-tui` | 872 lib passed; 0 failed（含新增 2） |
| `cargo test --workspace` | 1799 passed; 0 failed |
| `cargo clippy -p opencoder-tui --all-targets -D warnings` | 0 warnings |
| `cargo build --workspace` | Finished |

关键测试（新增，位于 `app_loop_dispatch_cmd_tests.rs`）：
- `tab_fill_input_adds_trailing_space` — Tab 选中 `/plan` 后断言
  `input == "/plan "`、`input.ends_with(' ')`、`input.starts_with('/')`、
  `cursor_idx == input.len()`、popup 关闭、未启动 turn、`LoopFlow::Redraw`。
- `tab_fill_local_command_adds_trailing_space` — 本地命令 `/ps` 同样获得尾随
  空格（`input == "/ps "`），覆盖命令类目一致性。

防修绿扫描：无删除 `#[test]`、无新增 `#[ignore]`、无 `assert!(true)` /
`is_ok()` / `is_some()` 弱断言；断言均为可观测值。

## 兼容性

- `FillInput` 枚举载荷语义不变；`command.rs` 单元测试不受影响。
- `dispatch()`（command.rs:189）不经 Tab 填充路径接收字符串（经
  `selected_action()`/`selected_name()`），不受影响。
- `FillInput` 仅有 app_loop.rs 一处消费点，无跨 crate 调用方。
