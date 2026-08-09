Commit: (working-tree, pre-initial-commit)

# feat(tui): 跨终端可靠的信息区文本选择/复制模式（Ctrl+G toggle）

## 背景

TUI 信息回显区的文本选择此前仅依赖 Kitty keyboard protocol 的 Shift 按住检测（`consume_modifier_or_release`
→ `shift_held`）。在 tmux / SSH / mosh / 非 Kitty 终端中，Shift 不产生可观测的键盘事件，
`shift_held` 永远为 false，原生文本选择不可用——且 tmux 的 `mouse` 选项会拦截拖拽，终端模拟器
根本看不到选择手势。

本变更引入一个**不依赖终端协议**的 copy/selection 模式：专用快捷键（默认 `Ctrl+G`，可配置）toggle
进入后，关闭 TUI 自身鼠标捕获 + 关闭 tmux `mouse` 拦截，将原始拖拽交还终端模拟器执行原生选择。
退出时恢复 TUI 鼠标捕获，但**保持 tmux mouse off**（避免终端/tmux 抢拖拽的死循环）。Kitty 终端
的 Shift 按住增强路径保留不变。

## 变更

- **`crates/tui/src/tmux_mouse.rs`**（新文件，125 行）：tmux `mouse` 选项协调。`disable()` 记录
  旧状态并 `tmux set mouse off`；`restore(prev)` 可选恢复（copy-mode 流不调用——保持 off）。
  纯函数 `parse_mouse` 覆盖 on/off/legacy-numeric/空白。仅 `$TMUX` 存在时 spawn tmux 子进程，
  否则短路。
- **`crates/tui/src/copy_mode.rs`**（新文件，130 行）：copy 模式逻辑封装。`enter()`=suspend 鼠标
  捕获 + tmux mouse off；`exit()`=resume 鼠标捕获（不恢复 tmux）；`is_active(cm, shift)`=
  `cm || shift`；`handle_key()` 处理 toggle + 活跃时吞掉所有按键（Esc 也退出）。纯决策逻辑
  提取为私有 `next_state()` 以满足零 I/O 单测。
- **`crates/core/src/config/keymap.rs`**：`KeymapConfig` 新增 `copy_mode` 字段（默认 `"ctrl+g"`），
  `KEYMAP_INFO` 加标签，`get`/`set` 加分支。绑定总数 19→20。
- **`crates/tui/src/keymap.rs`**：`KeyBindings` 加 `copy_mode: KeyCombo`，`from_config` 解析。
- **`crates/tui/src/app.rs`**：事件循环加 `copy_mode` 状态；`Event::Key` 中 `consume_modifier_or_release`
  之后调用 `copy_mode::handle_key`（toggle/吞键）；`Event::Mouse` 顶部以 `copy_mode::is_active`
  守卫短路所有点击交互；`render_frame` 调用透传 `copy_mode`。退出 tmux mouse 保持 off（用户决策）。
- **`crates/tui/src/frame.rs` / `render.rs`**：`render_frame`→`render`→`render_body` 透传
  `copy_mode: bool`。copy 模式激活时 body 边框改用 `warn_color()`，composer 区追加
  `COPY MODE: Ctrl+G/Esc` 状态 chip。
- **范围**：纯增量。2 个新模块 + 配置层 1 字段 + 事件循环 3 处守卫 + 渲染层视觉指示；
  未改 trait/数据形状/prompt 契约。现有 Shift-held Kitty 路径行为不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| is_active 真值表（copy_mode ‖ shift_held） | `is_active_truth_table` | `crates/tui/src/copy_mode.rs` |
| toggle 键翻转状态（Ctrl+G 进/出） | `toggle_key_flips_state` | `crates/tui/src/copy_mode.rs` |
| 活跃时吞掉普通按键 | `active_mode_swallows_other_keys` | `crates/tui/src/copy_mode.rs` |
| Esc 退出活跃模式 | `esc_exits_active_mode` | `crates/tui/src/copy_mode.rs` |
| 非活跃时普通键透传、toggle 仍生效 | `inactive_passes_through_non_toggle_keys` | `crates/tui/src/copy_mode.rs` |
| tmux mouse on/off 解析 | `parse_mouse_recognizes_on/off` | `crates/tui/src/tmux_mouse.rs` |
| legacy 数字/未知值不误恢复 | `parse_mouse_rejects_unknown_value` | `crates/tui/src/tmux_mouse.rs` |
| show-options 尾换行 trim | `parse_mouse_trims_trailing_newline` | `crates/tui/src/tmux_mouse.rs` |
| tmux 外 disable 短路返回 None | `disable_returns_none_outside_tmux` | `crates/tui/src/tmux_mouse.rs` |
| restore(None) no-op | `restore_none_is_noop` | `crates/tui/src/tmux_mouse.rs` |
| keymap 绑定数 = 20 | `keymap_info_count_matches_fields` | `crates/core/src/config/keymap.rs` |
| copy_mode 默认值 = ctrl+g | `default_values_match_documented_defaults` | `crates/core/src/config/keymap.rs` |
| keymap 菜单条目数 = 20 + 导航 wrap | `new_menu_has_20_entries` / `navigate_*_wraps` | `crates/tui/src/keymap_menu/state.rs` |

- 全量回归：`cargo test --workspace` → **2231 passed / 0 failed**
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）
- build：`cargo build --workspace` → 零错误（EXIT=0）
- 行数：`tmux_mouse.rs` 125（≤ 400，新增）；`copy_mode.rs` 130（≤ 400，新增）；`app.rs` 800（≤ 800）；
  `render.rs` 799（≤ 800）；`frame.rs` 261；`keymap.rs`(core) 190；`keymap.rs`(tui) 318
