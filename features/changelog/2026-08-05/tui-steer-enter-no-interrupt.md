# revert(tui): 键盘 Enter steer 不再中断运行中 turn，仅 admit

## 背景

08-04 的 `tui-alt-word-movement-and-steer-interrupt.md` 让键盘 Enter 与 `>`
按钮共享 `fire_steer_interrupt`，提交 steer 即 `fire_turn_cancel` 中断当前
turn。但键盘 Enter 与 `>` 的交互意图不同：Enter 是轻量 follow-up，应让运行中
turn 自然完成后在下一 idle/turn 边界吸收（runner `claim_steers` / late-steer
peek）；`>` 才是显式「立即打断」。两者共享一条 fire 路径把语义耦合死了。

## 变更

### `crates/tui/src/steer_fire.rs`

- 新增 `admit_keyboard_steer(store, sid, clean, display, pending_images,
  chat)`：键盘 Enter 路径专用——persist + 推 pending 面板，**函数签名不接
  `turn_cancel`**，结构上无法触发中断。图片仅在 store 写入成功后消费。
- 模块 doc 改写为两条路径的分叉说明（键盘仅 admit / `>` 独走中断）。

### `crates/tui/src/app.rs`

- `KeyAction::Steer` 分支：删除内联的 `admit_input` + `steer_items.push`
  重复块（×2：纯文本与纯 skill），改调 `steer_fire::admit_keyboard_steer`。
- `MouseOutcome::SteerSubmit`（`>` 按钮）保持调 `fire_steer_interrupt` 不变。
- runner 侧无需改动：idle 边界 `has_pending_steers` / `claim_steers` 自然吸收。

## 核心不变式

键盘 Enter 提交 steer 时 **不 fire turn_cancel**；运行中 turn 自然完成，steer
在下一 turn 边界被吸收。`>` 按钮是唯一立即中断的路径。

## 测试清单

| 路径 | 测试 | 文件 |
|------|------|------|
| tui | `keyboard_enter_admits_steer_without_firing_turn_cancel`（驱动真实 admit 缝，断言 `!is_cancelled()`，再对比 `>` 中断） | `steer_fire.rs` |
| tui | `enter_while_running_admits_steer`（键位分发返 `KeyAction::Steer`） | `app_tests/key_tests.rs` |
| tui | `only_button_path_interrupts_running_turn_with_steer` | `steer_fire.rs` |
| tui | `store_failure_returns_none_and_preserves_images`（注入失败 Store，断言返回 None + 图片不丢） | `steer_fire.rs` |

**当次实跑**: TUI steer 相关 **31 passed**（+2 新增）；TUI lib **887 passed**；
session steer_followup **8 passed**；workspace **1898 passed; 0 failed; 1 ignored**
（全量绿，无需排除）。`cargo clippy --workspace --all-targets -D warnings` clean。

> 注：修复 session WIP 的编译级联失败（`DrainOutcome` 缺 `#[derive(Debug)]` +
> `runner/mod.rs` 遗留未用导入 `claim_one_queued`/`has_pending_queues`/
> `has_pending_steers`），消除了 lib-test 无法编译导致的 11 个级联失败，使
> workspace 回归全绿。此修复属于 session WIP 的机械性收尾，非本变更的功能范围。


## 结构合规修复（go-live gate）

- `runner/mod.rs`（828→590 行）：抽出 `dedup_consecutive_bash_timeouts` +
  `dedup_tests` 到新模块 `runner/dedup.rs`（217 行），遵循既有 `pub(super)` +
  `use` 兄弟模块约定。
- `app_loop.rs`（818→649 行）：抽出粘贴/剪贴板逻辑（`route_paste` 等 4 函数）
  到新模块 `app_loop_paste.rs`（193 行），`pub(crate) use` 重导出保持调用点不变。
- `cli/run.rs`（811→778 行）：抽出 `load_image_data_uris` + `mime_from_ext` 到
  `run_image.rs`（41 行），`pub(crate) use crate::run_image::...` 重导出。
- 清理 `runner/mod.rs` 遗留未用导入（`has_pending_queues` / `has_pending_steers`）。
