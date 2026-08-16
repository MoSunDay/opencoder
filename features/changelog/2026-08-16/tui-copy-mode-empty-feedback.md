Commit: (working-tree, post-4174956)

# fix(tui): 空转录 Ctrl+G 静默死区 → 可感知反馈；摘除未实现的 hide_composer 绑定

## 背景

b663adb 的 copy 选区重构给 `copy_select::handle_key` 的进入条件加了 `total_rows > 0`
守卫，而空转录（新会话 tutorial 屏）在 `render.rs` 的 tutorial 早退路径上**永远走不到
ViewportCache 构建**，`total_rows` 恒为 0——Ctrl+G 被静默吞掉（无 flash、无 chip、
模式不进入），用户感知"快捷键没了"（旧 `copy_mode.rs` 是无条件翻转，空屏也能进模式
并见 chip）。且 `copy_select_tests.rs` 把"空 viewport → Ignored"作为期望行为锁进
测试，规范级回归被固化。取证另发现两处顺带缺陷：`KEYMAP_INFO` 广告了 TUI 从未实现
的 `hide_composer`（`KeyBindings` 无此字段、无 action）；keymap 数量注释漂移
（state.rs "21 entries"、keymap.rs "All 18"、实际 20/19）。

## 用户可见变更
- **空转录按 Ctrl+G**：键被吞但立即在 composer 状态区闪现 `empty — nothing to copy`
  chip（复用 mode_flash 通道，15 anim tick 自动消失）——不再静默无反馈；模式本身仍
  不进入（无内容可选）。
- **非空转录按 Ctrl+G**：行为不变（进入应用内选择模式）。
- **Ctrl+H 快捷键设置菜单**：20 条 → 19 条——摘除从未生效的 `hide_composer`
  （"Toggle bottom input"，TUI 无实现）；旧配置 JSON 残留的 `keymap.hide_composer`
  键被 serde 宽松忽略，无兼容性破坏（core 无 `deny_unknown_fields`）。

## 变更文件
- `crates/tui/src/copy_select.rs`：`CopyOutcome` 新增 `Empty`（toggle 命中但
  `total_rows == 0`：吞键 + 不进入）；新 `pub const EMPTY_FLASH_TEXT`；新
  `dispatch_key`（handle_key + apply_key + mode_flash 写入一站式接线，app 循环
  调用点收敛为单布尔返回）。
- `crates/tui/src/app.rs`：键事件 copy 分支改调 `dispatch_key`（净 -4 行，
  795 行守住 800 红线）。
- `crates/tui/src/copy_select_tests.rs`：翻转"空 viewport 拒入 = Ignored"断言 →
  `Empty`（含 viewport=None 的 tutorial 屏用例 + scroll/follow 不受扰）；新增
  `dispatch_key` 接线层测试与 flash 文案锁定。
- `crates/core/src/config/keymap.rs`：摘除 `hide_composer`（KEYMAP_INFO 条目 /
  KeymapConfig 字段 / default / get / set）。
- `crates/tui/src/keymap.rs`、`keymap_menu/state.rs`：注释与测试计数修正
  （"21 entries" → 19、"All 18" → 19、`new_menu_has_19_entries`）。

## 设计取舍
- 不采纳"tutorial 早退前构建 viewport"：空转录的 viewport 本就 0 行，构建了也进
  不了模式；且 render.rs 已 798 行逼近红线。反馈走 outcome 枚举层，覆盖**所有**
  空内容场景（tutorial 屏 / submitted 无块 / 空子代理会话），不止 tutorial 一处。
- `hide_composer` 摘除而非补实现：最小改动；广告不存在的功能比少一条可配置项
  伤害更大。

## 测试清单（功能 → 测试名）
- 空 viewport toggle → Empty（不进入、scroll/follow 不动）：
  `tui copy_select::tests::toggle_on_empty_viewport_flashes_instead_of_ignoring`
- viewport = None（tutorial 屏未建缓存）同样 → Empty：同上（第二断言组）
- flash 文案用户可读锁定：`tui copy_select::tests::empty_flash_text_is_user_facing`
- dispatch 接线层（空转录闪 flash 且 stamp anim_tick / 非 toggle 透传不闪 / 有内容
  进入不闪 / 二次 toggle 退出）：
  `tui copy_select::tests::dispatch_key_flashes_on_empty_and_passes_through`
- keymap 19 条契约：`core config::keymap::tests::keymap_info_count_matches_fields`；
  `tui keymap_menu::state::tests::new_menu_has_19_entries`
- 既有回归不破：`tui copy_select::tests::{toggle_key_enters_when_inactive,
  toggle_key_exits_when_active, inactive_passes_through_non_toggle_keys,
  active_mode_swallows_other_keys, esc_and_q_exit_active_mode}` 全绿

## 行数 gate

`copy_select.rs` 390、`copy_select_tests.rs` 246、`keymap.rs` 316、`keymap_menu/state.rs` 794、
`core/config/keymap.rs` 185（均 ≤800）；`app.rs` 795（净 -4，守住 800 红线）。

## 验证 gate

`cargo build --workspace` PASS · `cargo clippy --workspace --all-targets -- -D warnings` 0 警告 ·
`cargo test --workspace` 全绿（本轮用户已实测，授权免测提交；新增/翻转用例见上表）。
