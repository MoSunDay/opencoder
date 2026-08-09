Commit: (working-tree, uncommitted)

# feat(tui): /notepad — IDE 式查看/编辑视图（文件树 + vim 编辑器 + 终端 + rg/grep 搜索）

## 背景

TUI 缺少一个快速浏览/编辑 workdir 文件的入口。用户需要退出 TUI 或另开终端才能查看/修改
源码文件。本变更新增 `/notepad` slash 命令，激活一个全屏接管视图：左侧文件树、右侧 vim
编辑器（复用现有 `vim/` 引擎）、底部伪终端（`sh -c` 命令回显），并支持 `rg`/`grep` 文件
内容搜索。

## 变更

### 新增模块 `crates/tui/src/notepad/`（6 个文件，共 ~1929 行）

| 文件 | 行数 | 职责 |
| --- | --- | --- |
| `mod.rs` | 243 | `NotepadView` 结构体、`Focus`/`NotepadOutcome` 枚举、`new()`、`dispatch_key()` 异步包装、`render_frame()` 全屏渲染入口 |
| `tree.rs` | 358 | `TreeState`/`FlatNode`/`TreeInput`：递归扫描 workdir（跳过 `.git`/`target`/`node_modules`）、扁平化、展开/收起、导航、新建/删除文件 |
| `editor.rs` | 393 | `EditorState`：封装 `VimState`、文件加载/保存（`:w`/`:wq`）、带行号 gutter 渲染、光标定位 |
| `terminal.rs` | 257 | `TerminalState`/`TermLine`：命令日志、`run_command()`（`sh -c` + 10s 超时 + stdout/stderr 合并）、渲染 |
| `search.rs` | 357 | `SearchState`/`SearchHit`：`rg`（首选）/`grep -rn`（兜底）内容搜索、结果解析与导航、渲染 |
| `keys.rs` | 321 | 按焦点派发：Tree（j/k/Enter/n/d/H//）、Editor（vim 委派 + `:w` 拦截 + Esc 退出）、Terminal（输入/Enter/滚动）、Search overlay |

### 集成接线（4 处小改）

- **`command.rs`**（+4 行）：`COMMANDS` 增 `("/notepad", ...)`、`SlashAction` 增 `Notepad`、`parse()` 增 `"notepad"|"note"`、`dispatch()` 增 `"/notepad"`
- **`app.rs`**（798 行，净 +0）：压缩 `plan_edit` 按键拦截块（11→3 行）腾出空间，新增 `notepad` 声明、按键拦截、渲染短路、`/notepad` 自由文本分支、`dispatch_command` 参数
- **`app_loop.rs`**（780 行）：`dispatch_command` 增 `notepad` 参数 + `SlashAction::Notepad` match 分支；`LoopFlow` 增 `PartialEq, Eq` derive
- **`lib.rs`**（+1 行）：`pub mod notepad;`

### 关键设计

- **零新依赖**：文件树自实现递归；编辑器复用 `vim/` 引擎（`VimState` + `handle_vim_key`）；终端用 `tokio::process::Command`；搜索用 `rg`/`grep` 外部进程
- **焦点循环**：`Tab` 在 Tree→Editor→Terminal→Tree 间循环；Editor 的 Tab 仅在 Normal 模式触发（Insert 模式由 vim 处理）
- **Esc 语义**：Tree/Terminal 焦点 → 退出 notepad；Editor Normal 模式 → 退出；Editor Insert/Command/Search → 由 vim 处理
- **文件保存**：`:w` 在到达 vim 引擎前拦截（避免 "Unknown command" 提示），`:wq` 利用引擎 Exit + `is_modified()` 守卫自动保存
- **布局**：`[tree(30w) | editor]`（顶部 75%）+ `[terminal]`（底部 25%，min 6 行）；`H` 键切换树面板显隐；搜索 overlay 覆盖顶部

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| **tree.rs** | | |
| 树层级 + 噪声目录过滤 | `build_tree_hierarchy_and_filter` | `notepad/tree.rs` |
| 目录排在文件前 | `dirs_before_files` | `notepad/tree.rs` |
| 收起目录隐藏子项 | `collapse_hides_children` | `notepad/tree.rs` |
| 光标移动边界 | `move_cursor_bounds` | `notepad/tree.rs` |
| 重建保留收起状态 | `rebuild_preserves_collapse` | `notepad/tree.rs` |
| 滚动调整 | `ensure_visible_adjusts_scroll` | `notepad/tree.rs` |
| **editor.rs** | | |
| 加载文件到 VimState | `load_file_into_vim` | `notepad/editor.rs` |
| 保存写回磁盘 | `save_writes_to_disk` | `notepad/editor.rs` |
| `:w` 重置 modified | `do_write_resets_modified` | `notepad/editor.rs` |
| `:wq` 修改时写入 | `do_writequit_writes_if_modified` | `notepad/editor.rs` |
| `:wq` 未修改时跳写 | `do_writequit_skips_write_if_not_modified` | `notepad/editor.rs` |
| 行数计算 | `line_count_basic`、`line_count_empty` | `notepad/editor.rs` |
| 标题显示文件名 | `title_shows_filename` | `notepad/editor.rs` |
| `:w`/`:wq` 命令检测 | `is_write_cmd_detection` | `notepad/editor.rs` |
| 加载不存在文件 | `load_nonexistent_file` | `notepad/editor.rs` |
| **terminal.rs** | | |
| echo stdout | `run_command_echo` | `notepad/terminal.rs` |
| stderr 合并 | `run_command_stderr_merged` | `notepad/terminal.rs` |
| stdout+stderr 同时 | `run_command_stdout_stderr_both` | `notepad/terminal.rs` |
| 超时（10s） | `run_command_timeout` | `notepad/terminal.rs` |
| 空命令返回空 | `run_command_empty_returns_empty` | `notepad/terminal.rs` |
| 无输出 | `run_command_no_output` | `notepad/terminal.rs` |
| 独立进程 cwd | `run_command_independent_cwd` | `notepad/terminal.rs` |
| 日志裁剪 | `push_and_trim` | `notepad/terminal.rs` |
| 滚动边界 | `scroll_bounds` | `notepad/terminal.rs` |
| **search.rs** | | |
| 搜索找到内容 | `search_finds_content` | `notepad/search.rs` |
| 空查询返回空 | `search_empty_query_returns_empty` | `notepad/search.rs` |
| 无匹配 | `search_no_match` | `notepad/search.rs` |
| 多行文件 | `search_multiline_file` | `notepad/search.rs` |
| rg 行解析 | `parse_rg_line_basic`、`parse_rg_line_with_colons_in_text` | `notepad/search.rs` |
| 无效行解析 | `parse_rg_line_invalid` | `notepad/search.rs` |
| 结果光标边界 | `search_state_cursor_bounds` | `notepad/search.rs` |
| **keys.rs** | | |
| Tab 焦点循环 | `tree_tab_cycles_to_editor` | `notepad/keys.rs` |
| Esc 退出 | `tree_esc_exits`、`editor_esc_normal_exits`、`terminal_esc_exits` | `notepad/keys.rs` |
| 终端执行命令 | `terminal_runs_command` | `notepad/keys.rs` |
| 搜索→打开文件 | `search_finds_and_opens` | `notepad/keys.rs` |
| **mod.rs** | | |
| 初始 Tree 焦点 | `new_starts_in_tree_focus` | `notepad/mod.rs` |
| 空 editor/terminal | `new_has_empty_editor_and_terminal` | `notepad/mod.rs` |
| Esc 清除视图 | `dispatch_exit_clears_view` | `notepad/mod.rs` |
| Tab 循环 | `dispatch_tab_cycles_focus` | `notepad/mod.rs` |
| Enter 打开文件 | `dispatch_enter_opens_file` | `notepad/mod.rs` |

| **command.rs** | | |
| `/notepad` 解析 | `parse_notepad_full` | `command.rs` |
| `/note` 别名解析 | `parse_notepad_alias` | `command.rs` |
| `/notepad` 派发映射 | `dispatch_notepad` | `command.rs` |

## Gate（工作树实跑）

- `cargo test --workspace` → **2134 passed / 0 failed / 0 ignored**（47 个新测试）
- `cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告（EXIT=0）
- `cargo build --workspace` → 零错误（EXIT=0）
- 行数：所有新文件 ≤400（最大 `editor.rs` 393）；`app.rs` 798 ≤ 800；`app_loop.rs` 780 ≤ 800

## 限制

- 终端非真 PTY：不支持交互式程序（vim/top/REPL），仅一次性命令回显
- 文件监听：外部改动 notepad 不感知（切换文件时重读），可通过 mtime 检测改善
- 搜索：仅支持纯文本搜索（无正则），结果上限 200 条
