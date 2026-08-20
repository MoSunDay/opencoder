# TUI `@` file引用 pin 恢复 `@` 前缀（裸路径 pin 断裂提交时绝对路径展开链）

## 背景

上一轮将选择器 pin 的 token 改为裸 `relative/path `（见当日被替换的
`tui-file-mention-pin-bare-path` 记录），但提交侧权威展开点
`mention_resolve::expand_mentions` 只识别 **token 边界处的 `@` 前缀**——裸路径
token 提交时原样透传，"记录前重写为绝对路径"的链路在 TUI 真实路径上断裂：
picker pin 出的文件引用全部失去展开，且既有 e2e（`file_mention_flow.rs`）直接
以 `UiCmd::Prompt` 预置文本、绕过按键层，测试全绿而链路已断。本轮恢复
`@relative/path ` pin 语义（`@` 触发键仍被菜单消费，由 pick 重新带回前缀），
保持"输入框短形式、提交时绝对路径"的原设计自洽。

## 实现

- **`crates/tui/src/file_menu/state.rs`**：`Enter`/`Tab` 的 pick token 从
  `{rel} ` 恢复为 `@{rel} `（仍带尾空格，便于连续 pin 与下一个 token 边界）；
  `FileOutcome::Pick` 文档注释同步。
- **`crates/tui/src/key_handler.rs`**：触发分支的 token 起始判定提取为纯函数
  `pub fn char_opens_file_menu(input, cursor_idx, c) -> bool`（行为不变：
  行首或前一字符为空白处的 `@` 触发，email 等中间 `@` 不触发），触发分支与
  拦截分支注释同步为 `@relative/path ` 表述。pub 供 e2e 驱动生产判定而非复刻。
- **`crates/tui/src/file_menu/mod.rs` / 测试断言**：模块文档与
  `key_handler_file_mention_tests.rs`、`file_menu/state.rs` 断言恢复 `@` 前缀。
- **e2e 盲区封堵**：`crates/tui/tests/file_mention_flow.rs` 新增
  `picker_key_path_pins_expandable_mention`——经生产触发判定打开菜单、真实
  `FileMenu` 过滤 + `handle_file_key` Enter 选中、`composer::insert_str` 落入
  输入框，再走 `process_cmd(UiCmd::Prompt)` 提交，断言记录消息与模型请求均含
  绝对路径。今后 pick 再丢 `@` 标记会在 e2e 层直接红。

## 测试覆盖

| 功能 | 测试 | 文件 |
|------|------|------|
| Enter pick 输出 `@` 前缀 + 尾空格并关闭菜单 | `enter_picks_at_prefixed_token_with_trailing_space` | `crates/tui/src/file_menu/state.rs` |
| Tab pick `@` 前缀 / Esc 关闭 / 空查询 Backspace 关闭 | `tab_picks_esc_closes_backspace_on_empty_closes` | `crates/tui/src/file_menu/state.rs` |
| 按键全流程：过滤后 pick 将 `@token` 写入输入框 | `filter_then_pick_pins_token_into_input` | `crates/tui/src/key_handler_file_mention_tests.rs` |
| 触发判定纯函数（e2e 驱动的生产谓词） | `picker_key_path_pins_expandable_mention`（内联断言） | `crates/tui/tests/file_mention_flow.rs` |
| 按键路径 e2e：`@` 开菜单→Enter 选中→提交→记录/请求绝对路径 | `picker_key_path_pins_expandable_mention` | `crates/tui/tests/file_mention_flow.rs` |
| 手打 `@path` 提交展开（不回归） | `submitted_mentions_record_absolute_paths` | `crates/tui/tests/file_mention_flow.rs` |

## 回归

- `cargo test -p opencoder-tui --lib` → **1508 passed / 0 failed**（终验轮实跑；
  并行会话同时在增测，本轮先后两次门禁实跑 1505/1508 均全绿）
- `cargo test -p opencoder-tui --test file_mention_flow` → **2 passed / 0 failed**
  （含新增 `picker_key_path_pins_expandable_mention`）
- `cargo clippy -p opencoder-session -p opencoder-tui --all-targets` →
  **0 警告 / 0 错误**（首跑时并发会话 render.rs 的 `resolve_ctx_used` 转发
  短暂触发 1 条 unused-import 告警——非本变更文件；对方会话收敛后复跑为零）
- 附带复核：`cargo test -p opencoder-session --lib mention_resolve` → 11
  passed / 0 failed；`--test mention_expand` → 3 passed / 0 failed
