Commit: (working-tree, pre-initial-commit)

# test(tui): plan-edit popup Ctrl+C 退出行为的集成级回归测试

## 背景

`b6540ac`（`tui: restrict vim editor exits to :q/:q!/:wq`）修复了「Ctrl+C 关闭
plan-edit 弹窗」的问题：Insert 模式下 Ctrl+C 改为返回 `VimAction::Continue`
（留在弹窗、降回 Normal），Normal 模式下为 no-op，只有 Command 模式的
`:q`/`:q!`/`:wq` 才真正退出。该修复已提交，但缺少 **app_loop 集成层**的回归
覆盖。

vim 层单测（`vim/insert.rs`、`plan_edit.rs`）只能证明 Ctrl+C 在底层产出
`Continue`；若 `app_loop::handle_plan_edit_key` 里的 `take()`/restore 分支被
误调换，弹窗会静默关闭，而所有底层单测仍保持绿色——这正是需要补的缺口。

## 变更

### `crates/tui/src/app_loop_plan_edit_tests.rs`（新增，130 行）

独立测试模块，覆盖 `handle_plan_edit_key`（`pub(crate)`）的 `Continue` 与
`Exit` 两条分支。以 `#[path]` 内联挂载，镜像
`app_loop_session_only_tests.rs` / `app_loop_bugfix_tests.rs` 的既有约定。
使用 `ChatView::default()` + tokio channel，无 LLM / DB / 网络（<10ms）。

### `crates/tui/src/app_loop_tests.rs`（+4 行）

挂载新模块：
```rust
#[cfg(test)]
#[path = "app_loop_plan_edit_tests.rs"]
mod plan_edit_tests;
```

无 trait / 签名 / 配置 / 数据形状变更，纯测试增量，零生产调用点被触及。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Ctrl+C（modifier-chord 形式）保持弹窗开启，降回 Normal | `plan_edit_ctrl_c_chord_keeps_popup_open` | `crates/tui/src/app_loop_plan_edit_tests.rs` |
| Ctrl+C（原始 ETX `\u{3}` 形式）保持弹窗开启（raw-mode 终端实际投递的边界） | `plan_edit_ctrl_c_etx_keeps_popup_open` | `crates/tui/src/app_loop_plan_edit_tests.rs` |
| Esc 保持弹窗开启（`Continue` 分支正向对照） | `plan_edit_esc_keeps_popup_open` | `crates/tui/src/app_loop_plan_edit_tests.rs` |
| `:wq` 关闭弹窗并持久化修改后的文本（`Exit` 分支 + UiCmd::EditPlan 派发） | `plan_edit_wq_exits_and_persists_modified_text` | `crates/tui/src/app_loop_plan_edit_tests.rs` |

- 全量回归：`cargo test --workspace` → **1205 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误

## 范围说明

- 本轮仅提交测试增量（`app_loop_tests.rs` +4 / `app_loop_plan_edit_tests.rs` 新增）
  与本 changelog。
- 工作区中 `crates/session/src/tools/bash.rs` 的未暂存改动（移除 bash 工具 JSON
  schema 的 `timeout` 属性描述，属工具契约变更）**与本任务无关，已排除**。
