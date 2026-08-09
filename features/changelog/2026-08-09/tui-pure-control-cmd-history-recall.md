Commit: (working-tree, uncommitted)

# feat(tui): 纯控制命令（/plan、/act）idle 提交后进入方向键历史

## 背景

Enter/Tab 运行中提交、以及 Submit 路径的普通内容，都已写入 ↑/↓ 输入历史（见姊妹条目
`2026-08-08/tui-input-history-steer-queue-recall.md`）。但 **idle 下 Submit 一个纯控制命令**
（bare 的 `/plan`、`/act`、`/act_clear_context`）走的是 `is_pure_control` 分支：该分支为避免
回显 transcript marker、避免计入 `context_used`，原先**整段跳过**了 `push_user`，连带把方向键
历史也跳过——结果按 ↑/↓ 无法找回这类模式切换命令。本变更补齐这一缺口：纯控制命令同样进入
方向键历史，但**仍然抑制** transcript 回显与 context 计费。

## 变更

- **`crates/tui/src/app.rs`**（`run_app` → `KeyAction::Submit` 的 idle 分支）：
  将 `if !is_pure_control { push_user(...); context_used += ... }` 翻转为
  `if is_pure_control { push_history(&mut history, &mut hist_idx, &text) } else { push_user(...); context_used += ... }`。
  纯控制命令改走 `push_history`（只记历史、不回显、不计 context）；非纯控制路径行为不变。
- 行为契约：`/plan` 等纯控制命令 idle 提交后，↑ 可召回、↓ 可清空；transcript 与
  `context_used` 维持原状（不增）。
- **范围**：仅 `app.rs` 一处（+6 / −1，净增 5 行）。被调原语 `push_history` / `push_user` /
  `is_pure_control_cmd` 均已有单测覆盖；本迭代未新增测试（接线两侧的契约由既有单测锁定，见下）。

## 测试覆盖

本变更是对已充分测试原语的一行接线，**未新增测试**；下表列出锁定该接线两侧契约的既有单测：

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 纯控制命令分类：bare `/plan` 命中 | `bare_plan_is_pure` | `crates/tui/src/control_helpers_tests/is_pure_control.rs` |
| bare `/act`、`/act_clear_context` 命中 | `bare_act_is_pure`、`bare_act_clear_context_is_pure` | `crates/tui/src/control_helpers_tests/is_pure_control.rs` |
| 带参 / 普通文本 / 空串不命中 | `plan_with_plain_text_is_not_pure`、`plain_prompt_is_not_pure`、`empty_string_is_not_pure` 等 | `crates/tui/src/control_helpers_tests/is_pure_control.rs` |
| `push_history` 记录文本并重置 hist_idx | `push_history_records_text_and_resets_hist_idx` | `crates/tui/src/app_helpers_tests/mod.rs` |
| 多条按新→旧累积 | `push_history_accumulates_newest_last` | `crates/tui/src/app_helpers_tests/mod.rs` |
| 空文本仍记录、不留 stale 光标 | `push_history_records_empty_text_without_stale_cursor` | `crates/tui/src/app_helpers_tests/mod.rs` |
| `push_user` 仍记录历史 + 回显 marker（非纯控制路径不变） | `push_user_records_history_and_echoes_transcript` | `crates/tui/src/app_helpers_tests/mod.rs` |

## Gate（隔离工作树实跑，仅含本变更）

本仓库主工作树当前存在并发的其他在途改动（另一个在途特性），为剔除其干扰，以下数字取自一个
`git worktree`（基于 HEAD `4671385`，仅 apply 了 `app.rs` 的本变更）：

- 回归基线（HEAD，无本变更）：`cargo test --workspace` → **2084 passed / 0 failed / 0 ignored**
- 全量回归（HEAD + 本变更）：`cargo test --workspace` → **2084 passed / 0 failed / 0 ignored**（Δ = 0，无新增/删除测试 → 无回归）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）
- build：`cargo build --workspace` → 零错误（EXIT=0）
- 行数：`app.rs` 793 → **798**（≤ 800）；`app_helpers.rs` 698（未变，≤ 800）
