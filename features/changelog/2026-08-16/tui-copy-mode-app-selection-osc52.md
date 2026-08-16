# feat(tui): Ctrl+G 应用内选择模式 + OSC52 干净复制（替换"交还终端拖拽"方案）

## 背景

旧 copy 模式（2026-08-09）把选择手势交还终端模拟器：suspend 自身鼠标捕获 + tmux mouse off，
依赖终端原生选择与终端侧剪贴板。问题：选择/复制体验完全由终端决定（无法保证"干净"——
常带边框/装饰字符），SSH/tmux 下不可编程控制，且 app 无法感知选了什么。

本变更改为**应用内 vi 风格选择**：`Ctrl+G` 进入后 app 自己渲染光标/选区高亮，方向键/hjkl
移动，`v`/空格 锚定选区，`y`（留驻）或 `Enter`（复制即退出）通过 **OSC 52** 把**去装饰**
的文本写入系统剪贴板——跨终端、SSH 可用，无需本地剪贴板工具。

## 变更

- **`crates/tui/src/osc52.rs`**（新，137 行）：OSC 52 序列构造器。`sequence(text, in_tmux)`
  纯函数（base64 + `ESC ]52;c;…BEL`）；tmux 内自动 DCS passthrough 包裹（`ESC Ptmux;` +
  内层 ESC 全部翻倍 + `ESC \`）；`truncate_for_osc52` 以 100KiB base64 上限防超长序列被
  终端静默丢弃（UTF-8 字符边界截断）；`copy()` best-effort 写 stdout。
- **`crates/tui/src/copy_select.rs`**（新，347 行）+ `copy_select_tests.rs`/`copy_select_move_tests.rs`：
  - `CopySel { cursor, anchor, copied_at }`：绝对内容行坐标（屏幕行+scroll），选区随滚动
    保持锚定；`row_range()` 归一化 (lo,hi)；`flash_active/chip_text` 驱动 COPY/COPIED chip。
  - `handle_key`（迁移并扩展旧 `copy_mode::handle_key` 契约）：toggle 翻转、活跃吞键、
    Esc/q 退出、inactive 透传；移动键全表 `↑↓←→/hjkl/g/G/Home/End/PageUp/PageDown/Ctrl+b/f`，
    `h/l` 在折行逻辑行首/行尾间跳；`v`/空格 切换锚点；`y` 复制留驻、`Enter` 复制退出。
  - `ensure_visible`：光标移出视口时调 scroll 并清 follow（clamp 到 max）。
  - `yank_text`：选区行范围 → 整逻辑行；**折行重 join**——跨多屏行的逻辑行复制为单行，
    永不在 wrap 点断行（含回归用例）；`strip_decor` 剔除 `❯ User:/❯ Say:` 头、4 空格内容
    gutter、水平分隔线（≥3 连字符/横线字符）。
  - `highlight_lines`：选中逻辑行 span 打 `theme::highlight_bg()`；无选区时光标行加下划线。
  - `apply_key`：Yank/YankExit 时执行 `osc52::copy` + 记 flash，Exit 清模式（app 循环瘦身）。
- **`crates/tui/src/render_viewport.rs`**：新增 `line_at_row(row)`——`cum_rows` 二分反查
  绝对屏行所属逻辑行（`row_of_line` 的逆），O(log n)。
- **`crates/tui/src/render.rs` / `frame.rs`**：`copy_mode: bool` 参数改 `copy_sel: Option<&CopySel>`；
  body 边框 warn 高亮保留；可见行经 `highlight_lines` 后再入 Paragraph；composer chip 由
  `chip_text` 驱动（COPIED 时 ok_color，否则 warn_color）。
- **`crates/tui/src/app.rs`**（800→790 行）：`copy_sel` 状态接入 `Event::Key`（copy 路由在
  modal 之前，与旧 copy_mode 同优先级）与 `Event::Mouse`（`is_active(sel, shift)` 守卫）；
  为守住 800 行上限下沉三块：Paste 事件 → `app_loop_paste::handle_paste_event`、mouse
  steer-submit 收尾 → `app_loop_actions::steer_submit_after_mouse`、双 Esc 硬中断 →
  `app_loop_actions::cancel_running_turn`（app_loop.rs 741 行、app_loop_actions.rs 358 行）。
- **删除 `crates/tui/src/copy_mode.rs`**：全部决策/测试迁入 copy_select；`tmux_mouse` 模块
  保留（pub，备用）。旧"活跃时 suspend 鼠标捕获"语义取消——应用内选择不需要交还拖拽。
- **`crates/tui/src/keymap_menu/help.rs`**：Ctrl+G 文案改为「应用内选择模式: ↑↓←→/hjkl 移动,
  v 选区, y/Enter 复制(OSC52), Esc 退出」；鼠标节删除过时的「拖拽选择文本并复制到剪贴板
  （OSC52）」，SHIFT+拖拽 标注为终端原生选择；新增 `help_copy_mode_text_is_current` 锁定。

## 用户可感知变化

- `Ctrl+G` 进入应用内选择：光标行下划线 → `v` 后拖出 `highlight_bg` 选区 → `y`/`Enter`
  复制，chip 显示 `COPIED (OSC52)`（ok 色，2s）；粘贴内容不再带 `❯ Say:`/缩进/分隔线。
- 复制在 SSH/tmux 内开箱可用（tmux 需 `set-clipboard on` 或 ≥3.3 passthrough）。
- 超大选择（>75KiB 文本）自动截断而非整段丢弃。

## 测试清单（+33 新 / −5 旧迁移，全量 2729 passed / 0 failed，165 suites）

- `osc52::tests`（6）：plain/tmux 包裹格式（内层 ESC 全成对）、unicode/空串、
  截断字符边界与 100KiB 上限。
- `copy_select::tests`（11）：is_active 真值表、entry 状态、row_range 归一化、
  toggle 进入/退出、活跃吞键、Esc/q 退出、inactive 透传、空视口拒入、v/空格锚点切换、
  y/Enter 语义。
- `copy_select::tests::movement`（13）：箭头/hjkl 移动与两端 clamp、Page/Home/End、
  折行行内 h/l 跳、ensure_visible 滚动+follow 清除、strip_decor 全表、无选区 yank 当前行、
  多行 join、**折行重 join 回归**（跨行选/中段选均单行输出）、双折行选区、空视口 None、
  chip 阶段、highlight 选区/光标/保留原样式。
- `render_viewport::tests`（2）：`line_at_row` 折行映射 + 越界 clamp、空视口。
- `keymap_menu::help::tests`（1）：`help_copy_mode_text_is_current` 文案锁定。
- 迁移删除：旧 `copy_mode` 5 个用例（真值表/toggle/吞键/Esc/透传）语义并入 copy_select。

## 行数 gate

`osc52.rs` 137、`copy_select.rs` 347、`copy_select_tests.rs` 198、`copy_select_move_tests.rs` 293
（均 ≤400 新文件上限）；`app.rs` 788、`app_loop.rs` 741、`app_loop_actions.rs` 358、
`app_loop_paste.rs` 266、`render.rs` 791、`render_viewport.rs` 192、`frame.rs` 132（≤800）。

## 验证 gate

`cargo build --workspace` PASS · `cargo clippy --workspace --all-targets -- -D warnings` 0 警告 ·
`cargo test --workspace` 2729/0（基线 2694 + 本变更 28 + 仓库中并行 todos WIP 7）。
