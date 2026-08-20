# TUI `@` 文件引用：输入框 file-mention 选择器 + 提交时绝对路径展开

## 背景

用户在提示中引用仓库文件时只能手打路径，且相对路径对模型与回放视图都存在歧义。
本特性引入 `@` file-mention：输入框内 **token 起始处**（光标在行首或前一字符为空白）
键入 `@` 打开基于工作目录的文件弹出选择器；选中即把 `@relative/path `（带尾空格）
写入输入框（"pin"），重复操作可 pin 多个文件。提交时，凡是解析为工作目录下真实
存在文件/目录的 `@path` token 都在 **记录前** 重写为绝对路径——存储与渲染的用户
消息（以及发给模型的请求）展示完整路径；非路径 token（`a@b.com` 邮箱、`@param`、
不存在的名字、`@/etc/passwd` 绝对路径、`@../x` 逃逸）保持原样。

## 实现

- **会话侧（权威展开点）**：新模块 `crates/session/src/mention_resolve.rs`
  （纯函数，≤200 行含测试）：`expand_mentions(text, workdir)` 扫描 token 边界处的
  `@`，贪心收集 `[A-Za-z0-9._/-]` 候选，`try_exists` 命中即替换为 canonical 绝对
  路径；句尾标点（`.` `,` `;` `:` `!` `?`）通过逐字符回退重试保留原样；绝对路径
  与 `..` 逃逸候选直接拒绝。两个两行钩子接入（**不触碰已超限的 runner/mod.rs**）：
  - `skill_resolve::resolve_inline_skills` 尾部 —— 覆盖直接提交路径（TUI 提交、
    headless `opencode run`、web prompt 全部经 runner `run`）。
  - `skill_resolve::record_compound` 头部 —— 覆盖 steer 提升与 queue 排空路径
    （与 plan_tag 的入口覆盖面一致）。
- **TUI 选择器**：新模块 `crates/tui/src/file_menu/`（镜像 `command.rs` 的 `/`
  选择器结构）：`list.rs` 经 `ignore::WalkBuilder` 收集工作目录条目（隐藏/`.git`/
  gitignore-aware、深度 8、上限 2000 条、字典序）；`state.rs` `FileMenu` +
  `FileOutcome{Idle,Pick,Close}` + `handle_file_key`（字符追加查询并模糊过滤——
  复用 `menu::fuzzy_score`；↑/↓ 环绕导航；Enter/Tab=Pick；Esc/空查询 Backspace/
  Ctrl-D=Close）；`view.rs` `render_file_popup` 锚定 composer 上方（几何同
  `render_command_popup`）。
- **接线**：`key_handler.rs` 新增 `file_menu`/`workdir` 参数——菜单打开时按键全部
  拦截（镜像 `$` skill 菜单），`Pick` 经 `composer::insert_str` 插入 token 并记
  undo 快照；`Char('@')` 仅在 token 起始处触发（email 中间的 `@` 不触发、原样插
  入）。`app.rs` 持有 `Option<FileMenu>` 并串联渲染。
- **文件规模红线**：`app.rs`（800 上限）通过把 QueueUnsupported 提示文案移入
  `app_helpers::queue_unsupported_flash` 腾出空间；`render.rs` 把弹出层集群原样
  提取为 `render_popups.rs`（`#[path]` 约定，同 `render_status.rs`）。

## 对齐决策

1. **Pin 交互**：Enter 插入单个 token 并关闭菜单（与 `$skill` 选择器一致）；
   再次键入 `@` pin 下一个文件。
2. **输入框形态**：显示 `@relative/path ` 短形式（可读），仅提交时展开为绝对路径。
3. **展开位置**：session-runner 侧（headless/web 一并覆盖），非 TUI 本地展开。

## 测试覆盖

| 功能 | 测试 | 文件 |
|------|------|------|
| 展开：存在文件/目录/嵌套/多重/行首 | `expands_*` / `leading_mention_at_start_of_text` | `crates/session/src/mention_resolve.rs` |
| 展开：不存在 token 原样、email、`@param`、句尾标点、绝对/逃逸拒绝、裸 `@`、`@...` | `nonexistent_token_stays_verbatim` 等 | 同上 |
| 三个 runner 入口展开（direct/steer/queue） | `direct_prompt_expands_mentions` / `steer_prompt_expands_mentions` / `queued_prompt_expands_mentions` | `crates/session/tests/mention_expand.rs` |
| 遍历：排序、隐藏/.git/gitignore 跳过、上限、深度、缺失目录 | `lists_files_and_dirs_sorted` 等 5 例 | `crates/tui/src/file_menu/list.rs` |
| 选择器状态：打开/过滤/导航环绕/Pick/Esc/Backspace/Ctrl-D | `opens_with_all_rows…` 等 6 例 | `crates/tui/src/file_menu/state.rs` |
| 按键流：token 起始触发、email 不触发、过滤选中 pin、Esc 取消 | `at_at_token_start_opens_menu_without_inserting` 等 5 例 | `crates/tui/src/key_handler_file_mention_tests.rs` |
| TUI worker 全链路：提交后记录/请求/存储均为绝对路径 | `submitted_mentions_record_absolute_paths` | `crates/tui/tests/file_mention_flow.rs` |
| TUI 按键路径 e2e：`@` 开菜单→Enter 选中→提交→绝对路径 | `picker_key_path_pins_expandable_mention` | `crates/tui/tests/file_mention_flow.rs` |

## 回归

`cargo test -p opencoder-session -p opencoder-tui`（lib + 全部集成测试）通过；
全量 `cargo test --workspace --no-fail-fast` → **201 suites / 3185 passed / 0 failed**
（2026-08-20 01:02 实跑，独立 target 目录避开并发会话锁竞争；clippy 两 crate 0 警告）。
