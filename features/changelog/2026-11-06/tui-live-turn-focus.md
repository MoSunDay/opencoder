# 直播位恢复整 turn 展示（TUI）

一句话：撤销同日 `running_step_only`「运行位只画最新 step」的口径——直播
运行位自动展开后渲染**整个**最新 turn 的全部 step 行（已完成 step 折叠为
`▸ Step(i)` 单行，仅运行中的最后 step streaming thinking + calls），Say /
Done / Error settle 后整 turn 折回 `N Steps + Say` 稠密摘要；steps 在其余
位置（回放、已结算 turn）默认折叠，只有用户点击展开（`user_touched` pin
不变）。

## 行为口径

- 运行位（`step_live::activate` 自动打开的组行）渲染当前 turn 的每一个
  step 行：历史 step 是折叠单行，运行位是唯一展开 streaming 的行；组行
  `❯ N Steps` 计数与 running spinner 提示不变。
- settle（Say 首块 / `Done` / `Error` / 新 turn 准入）把整 turn 折回
  `Say(n steps): <preview>` 合并对（或折叠组行），step 行全部隐藏；用户
  pin 过的层级照旧不被自动收起。
- `user_touched` pin 语义不变：用户点开/收起的组行与 step 不再被自动
  披露移动；用户展开的阶梯同样渲染全部 step 行。
- 删除 `running_step_only` / `first_visible_step` 谓词：渲染器
  （`chat_step_render::flatten_step_group`）、行号镜像
  （`chat_headers::collect_headers`）、点击目标
  （`chat_steps::visible_targets`）三处恢复遍历全部 step，行号镜像天然
  对齐，无需共享 skip。
- 与 SPA `steps/useLadderDisclosure.js` 回到同一可见性口径（SPA 本就为
  每个 step 渲染折叠头行）；SPA 无需改动。

## 变更清单

- `crates/tui/src/chat_step_live.rs` — 删除 `running_step_only` /
  `first_visible_step` 及其内联测试；模块文档恢复整 turn 披露口径
  （`activate` / `settle` / `settle_steps_before_last` 逻辑不变）。
- `crates/tui/src/chat_step_render.rs` — `flatten_step_group` 去掉
  `user_touched` 参数与首行 skip，遍历全部 step；模块文档与 LIVE FOCUS
  注释同步。
- `crates/tui/src/chat_headers.rs` — `collect_headers` 的 StepGroup 分支
  恢复对全部 step 计数行号与 `call_idx`。
- `crates/tui/src/chat_steps.rs` — `visible_targets(open, steps)` 恢复
  双参签名与内联测试；`settle_steps_before_last` 及其在
  `append_step_call` 的调用保持不变。
- `crates/tui/src/chat_flatten.rs`、`crates/tui/src/chat.rs` — 调用点去掉
  `user_touched` 传参。
- `crates/tui/src/chat_types.rs` — `StepGroup` 的 LIVE DISCLOSURE 文档
  恢复整 turn 口径。
- `crates/tui/src/chat_tests/line_accounting.rs`、
  `chat_tests/line_accounting/step_ladder.rs`、
  `chat_tests/step_group/live_disclosure.rs` — 行数镜像与披露测试恢复
  整 turn 断言。
- `agents/tui/index.md` — `chat_step_live.rs` 条目口径同步。

> 撤销映射：`live_running_position_renders_only_the_last_step` /
> `live_settle_folds_the_running_step_back_into_the_say_pair`
> （`tui-live-step-focus.md`，同日）→
> `live_running_position_renders_the_whole_turn` /
> `live_settle_folds_the_whole_turn_back_into_the_say_pair`（本次）。历史
> changelog 保持原样。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 运行位渲染整个 turn：每个 step 行都在、仅运行位 streaming | `live_running_position_renders_the_whole_turn` | `crates/tui/src/chat_tests/step_group/live_disclosure.rs` |
| settle（Say 首块）折回 `Say(n steps)` 摘要、step 行消失 | `live_settle_folds_the_whole_turn_back_into_the_say_pair` | `crates/tui/src/chat_tests/step_group/live_disclosure.rs` |
| 用户 pin 的展开阶梯同样渲染全部 step 行 | `user_pinned_open_ladder_renders_every_step_row` | `crates/tui/src/chat_tests/step_group/live_disclosure.rs` |
| 自动态行号镜像对齐：全部 step 行入账（flatten ↔ headers） | `auto_running_position_keeps_alignment` | `crates/tui/src/chat_tests/line_accounting/step_ladder.rs` |
| `visible_targets` 展开态暴露全部 step / calls / call 目标 | `visible_targets_mirror_the_three_level_ladder` | `crates/tui/src/chat_steps.rs` |
| 用户展开阶梯全深度行号对齐（回归保护） | `step_group_ladder_depth_alignment`、`step_group_with_expanded_call_keeps_alignment` | `crates/tui/src/chat_tests/line_accounting/step_ladder.rs` |

定向回归（测试范围仅本次改动链路，未跑全量）：

- `cargo test -p opencoder-tui --lib` → 1816 passed / 1 failed。唯一失败
  `app::subagent_input::tests::store_failure_preserves_pending_images` 为
  本工作区既有问题：他人未提交的 `TursoStore`→`RdbStore`（rdb 原型）
  替换移除了 `TursoStore`，RdbStore 原型不按 FK 拒绝写入；干净 worktree
  @ `a6548ed1b` 上同一测试通过，与 step ladder 改动无关。
- `cargo clippy -p opencoder-tui --all-targets` → 本次改动文件零警告；
  唯一 `opencoder-tui` 告警（`ts_mirror_tests.rs` 的
  `suspicious_open_options`）同样来自工作区未提交的 ts_registry 原型，
  另有 `opencoder-core` / `opencoder-session` 各 1 条既有基线告警。
- `grep -rn "running_step_only\|first_visible_step" crates/` → 无命中。
