Commit: (working-tree, pre-initial-commit)

# feat(tui): notepad 改为分屏布局 + composer 内 `!cmd` 本地 bash 执行

## 背景
notepad 此前为全屏接管（三面板：文件树 + vim 编辑器 + 底部 vim 控制台），控制台复刻了一套独立
的 composer/vim/echo/bash 链路，与主 chat composer 双轨并行、维护成本高，且全屏接管切断了
用户与正在运行的会话流的视线。本次将 notepad 降格为「上半区分屏」：顶部仍是文件树+编辑器，
底部直接复用主 chat body + composer；原控制台的 `!cmd` 本地 bash 能力上移到主 composer
（`!` 前缀），消除独立控制台这条重复链路。**Supersedes** `tui-notepad-vim-console.md`（控制台已删除）。

## 变更

### notepad：全屏接管 → 分屏（上半区 tree/editor，下半区 chat）
- **`crates/tui/src/notepad/mod.rs`**：`Focus` 枚举删去 `Console` 变体（剩 `{Tree, Editor}`）；
  `NotepadView` 删去 `console`/`console_hidden` 字段，新增 `height: u16`（上半区高度，可调）。
  `NotepadOutcome` 由四变体重构为 `{Exit, Consumed, FocusChat}`（去 `SubmitPrompt`/`RunBash`——
  bash/prompt 现走主 composer）。新增 `render_top()`（把 tree/editor 画进给定 rect）、
  `layout_split(area, height)`（切出 top/divider/bottom 三 rect，divider 固定 1 行）、
  `render_divider()`（可拖拽分隔条）。不再有独立 `render_frame` 全屏绘制。
- **`crates/tui/src/notepad/keys.rs`**：删除全部 console 按键处理；`Tab` 改为 Editor→Tree 循环
  （原先 →Console）。修复 `start_create` 父目录解析（选中文件时取其父目录）。
- **删除 `crates/tui/src/notepad/console/`**（`mod.rs`/`render.rs`/`state.rs`/`submit.rs`，
  共 ~640 行 `ConsoleState`/`EchoLog`/`SubmitKind`/`render_console`）与
  **`crates/tui/src/notepad/terminal.rs`**（~257 行 `sh -c` 执行助手）——能力已上移。

### composer 内 `!cmd` 本地 bash（替代原控制台 bash）
- **`crates/tui/src/bash_exec.rs`**（新增 104 行）：纯函数 `run_command(cmd, workdir)`
  （`sh -c` 合并 stdout+stderr，10s 超时，空输出→`(no output)`）+ `spawn(cmd, workdir)`
  （`tokio::spawn` + oneshot，UI 不阻塞）。
- **`crates/tui/src/key_handler.rs`**：新增 `KeyAction::Bash(String)`；Enter 时若输入以 `!` 开头，
  去前缀 trim 后返回 `Bash`（运行中亦可用）。
- **`crates/tui/src/chat_helpers.rs`**：`ChatView::push_bash_tool(cmd)`（推一个展开的
  `ChatBlock::Tool` 占位）、`finish_bash_tool(output)`（回填输出、折叠、记录耗时）。
- **`crates/tui/src/app_notepad.rs`**（重写 336 行）：`handle_bash`（记录历史 + spawn + push 占位块）、
  `poll_bash`（try_recv 完成块 / 中断时回填 `(command aborted)`）、`handle_notepad_drag`
  （鼠标拖拽 divider 调整 `height`）、`key`（Ctrl+O 焦点切换 notepad↔composer）。

### keymap：`toggle_console` → `toggle_focus`
- **`crates/core/src/config/keymap.rs`** / **`crates/tui/src/keymap.rs`**：字段/键名
  `toggle_console`→`toggle_focus`，默认绑定 `Ctrl+Shift+T`→`Ctrl+O`。语义由「显隐控制台面板」
  改为「notepad 顶部 ↔ 主 composer 焦点切换」。

### 渲染：notepad 下半区 = chat
- **`crates/tui/src/render.rs`**：`render()` 新增 `notepad: Option<&NotepadView>` 参数；notepad
  打开时把 chat 画进下半区 rect（`draw_area`），上半区由 `notepad::render_top` 绘制。
  抽出 `render_status()` 到 **`crates/tui/src/render_status.rs`**（新增 83 行）以保持本文件 ≤800。
- **`crates/tui/src/frame.rs`** / **`crates/tui/src/app_loop.rs`** / **`crates/tui/src/app.rs`**：
  透传 `notepad` 与 `np_chat_focus`/`np_drag`/`bash_rx` 局部态；移除原 `app_notepad::try_render`
  全屏分支与 `console.set_running` 调用。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| `!cmd` 前缀返回 Bash action | `bang_prefix_returns_bash_action` | `crates/tui/src/key_handler_tests.rs` |
| `!cmd` 带空格仍识别 | `bang_prefix_with_spaces_returns_bash` | `crates/tui/src/key_handler_tests.rs` |
| 单独 `!` 为 noop | `bare_bang_is_noop` | `crates/tui/src/key_handler_tests.rs` |
| 运行中 `!cmd` 仍可用 | `bang_prefix_works_while_running` | `crates/tui/src/key_handler_tests.rs` |
| run_command 捕获 stdout/stderr/空输出/超时 | `run_command_captures_stdout` 等 5 例 | `crates/tui/src/bash_exec.rs` |
| push_bash_tool 创建展开 Tool 块 | `push_bash_tool_creates_expanded_tool_block` | `crates/tui/src/chat_tests/tool_collapse.rs` |
| finish_bash_tool 回填+折叠 | `finish_bash_tool_fills_output_and_collapses` | `crates/tui/src/chat_tests/tool_collapse.rs` |
| 中断 bash 回填信息 | `finish_bash_tool_aborted_message` | `crates/tui/src/chat_tests/tool_collapse.rs` |
| Ctrl+O 切换 notepad↔chat 焦点 | `toggle_focus_switches_to_chat` 等 2 例 | `crates/tui/src/app_notepad.rs` |
| Esc 关闭 notepad | `esc_closes_notepad` | `crates/tui/src/app_notepad.rs` |
| chat 聚焦时非切换键 fall through | `keys_fall_through_when_chat_focused` | `crates/tui/src/app_notepad.rs` |
| divider 拖拽调整高度/钳制下界 | `drag_starts_on_divider_click` 等 5 例 | `crates/tui/src/app_notepad.rs` |
| tree/editor Tab 双面板循环（无 Console） | `editor_tab_cycles_to_tree` 等 | `crates/tui/src/notepad/keys.rs` |
| 编辑器保存/退出 e2e（双面板） | `edit_and_save_with_colon_w` 等 | `crates/tui/tests/notepad_edit_flow.rs` |
| 搜索 + tree 隐藏 e2e（新布局） | `search_open_loads_file` 等 | `crates/tui/tests/notepad_search_terminal.rs` |

## Gate
- 全量回归：`cargo test --workspace` → 2219 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → Finished
- 行数：`bash_exec.rs` 104 ≤ 400；`render_status.rs` 83 ≤ 400；`app_notepad.rs` 336 ≤ 800；
  `app.rs` 799 ≤ 800；`app_loop.rs` 797 ≤ 800；`key_handler_tests.rs` 799 ≤ 800

## Impact Surface
- 用户可感知：`/notepad` 不再全屏接管，改为可调高的上半区 + 下方继续显示会话流；`Ctrl+O`
  在 notepad 与输入框间切焦点；在输入框打 `!ls` 等可直接执行本地命令并以可折叠 Tool 块回显。
- 配置兼容性：`keymap.toggle_console` 改名为 `toggle_focus`，旧配置键名失效（值需迁移到新键名）。
- 不影响：CLI/Web/session/store/llm 边界；worker 通道与 SessionEvent 模型不变。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
- Supersedes [notepad vim 控制台](tui-notepad-vim-console.md)
- 前身 [notepad IDE 视图](tui-notepad-ide-view.md)
