Commit: (working-tree)

# feat(tui): copy 模式回归终端原生选择 + 正文去装饰全宽净化视图；移除 OSC52

## 背景

b663adb 把 Ctrl+G 改成 app 内 vi 风格选区（光标行走 + v/y + OSC52 写剪贴板）。
用户实测后要求回归旧行为并解决"终端复制拿到脏文本"：终端原生拖拽选择复制到
的内容里混着渲染装饰（body 圆角边框、滚动条列、`[turn cost]` 行、`❯ User:/Say:`
角色头、`───` 分隔线、4/2 空格缩进槽、代码块 `┌ lang`/`│ `/`└───` 边框前缀）。
本变更：① 回归"交还终端"旧契约；② copy 激活期间正文按无装饰全宽渲染，让终端
自己的复制快捷键拿到干净文本；③ app 不再写剪贴板，复制完全交给终端快捷键。
取代 [app 内选区+OSC52](tui-copy-mode-app-selection-osc52.md) 与
[空转录 Empty flash](tui-copy-mode-empty-feedback.md)（Empty flash 随新模式一并移除）。

## 用户可见变更
- **Ctrl+G 进入 copy 模式**：暂停 TUI 鼠标捕获 + 关闭 tmux `mouse` 拦截，原生拖拽
  选择交还终端模拟器（任意终端）；Esc/Ctrl+G 退出并恢复捕获（tmux 保持 off，旧语义）。
- **copy 模式期间正文净化渲染**：无 body 圆角边框、无滚动条列、无 `[turn cost]` 行、
  无边框行指示器（跟随/跳顶箭头），全宽显示；行内剥掉缩进槽与代码框前缀，丢弃角色头/
  分隔线/代码框边框行——终端自带复制快捷键粘贴结果无 `│`/`┌`/gutter/边框残留。
  语义信息保留：`▸ 工具`头、`💭 Thinking` 等语义头、槽位之外的相对缩进、空行间距。
- **移除 OSC52**：app 不再写剪贴板（`osc52.rs` 删除）；空转录 Ctrl+G 回归无条件进入
  （chip 提供反馈），`empty — nothing to copy` flash 不复存在。
- composer 状态 chip 恢复 `COPY MODE: Ctrl+G/Esc`；帮助页（Ctrl+H → 帮助）文案重写
  为"交还终端原生拖拽选择 + 去装饰全宽显示，用终端快捷键复制"。

## 变更文件
- 删除：`copy_select.rs`（app 内选区）、`copy_select_tests.rs`、
  `copy_select_move_tests.rs`、`osc52.rs`。
- 新增 `crates/tui/src/copy_mode.rs`（323 行 ≤400 红线）：恢复旧契约
  `enter()/exit()/is_active(bool,bool)/next_state/handle_key`；新增纯函数清洗层
  `clean_text(&str) -> Option<String>`（`strip_slots` 剥 4/2 空格槽 + `│ `/`│`/`▎ `
  前缀；丢弃 `❯ User:/Say:`、`is_separator` 分隔线、`┌`/`└` 代码框行）+
  `render_clean(...)`（全宽无框渲染 visible 窗口；复用 `ViewportCache`，宽度差异
  自然触发进出模式时的重建；首行被清洗丢弃时同步丢弃 `top_skip` 防错位跳行）。
- `app.rs`（-10 行）：`copy_sel: Option<CopySel>` → `copy_mode: bool`；dispatch
  收敛回 `handle_key` 单行；Mouse 守卫换 `copy_mode::is_active`。
- `render.rs`（+1 行，799 ≤800 红线）：render_body 头部 `copy_mode` 早分支走
  `render_clean`（跳过边框/滚动条/timer/指示器与 hit-rect 记录）；chip 恢复固定
  文案；移除选区高亮与边框高亮。
- `frame.rs`：参数改名 `copy_sel` → `copy_mode: bool`。
- 测试接线：`render_tests/body.rs`（10 处）/`timer.rs`（3 处）render_body 末参
  `None` → `false`；全量 `render()` 调用点（chips/arrow_click/cursor_popup/
  render_clear_tests）同步。

## 测试

- `copy_mode` 单测 10 条（全绿）：恢复旧 5 条（is_active 真值表 / toggle 翻转 /
  活跃吞键 / Esc 退出 / 非激活透传）+ 新增 4 条 `clean_text` 规则（头/分隔线/
  代码框丢弃；gutter 与 `│ `/`▎ ` 前缀剥离且深缩进保留；语义头保留）+
  1 条 `render_clean` 端到端（TestBackend：代码文本存活、`┌`/`└`/`│`/`❯ Say:`
  全无）。
- `keymap_menu/help`：`help_copy_mode_text_is_current` 断言重写（锁定"终端原生拖拽
  选择/去装饰"、禁止 OSC52/应用内选择模式文案回归）；快捷键菜单 19 条计数契约不变。
- 回归：`cargo test --workspace` 全绿；`cargo clippy --workspace --all-targets
  -- -D warnings` 零警告；`cargo build --workspace` 干净。

## Memory 同步

- `agents/tui/index.md`：`copy_select` 条目重写为 `copy_mode`（终端原生契约 +
  清洗层 + 模块删除说明）。
- `features/index.md`：copy 模式条目更新（净化视图 + 新 changelog 链接）；顺带修正
  stale 的"20 个快捷键"→ 19。
