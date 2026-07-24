Commit: (working-tree, pre-initial-commit)

# fix(tui): Ctrl+U 与 Ctrl+L 行为统一

## 背景
此前 Ctrl+U 是「向上滚动 10 行」的快捷键（`apply_scroll` 内 `scroll - 10`），
而 Ctrl+L 是「折叠全部 thinking / 退出 subagent 视图 / 清空输入框」。两者语义
无关，用户难以记忆，且 Ctrl+U 的滚动语义与 PageUp 高度重叠。需求：让 Ctrl+U
与 Ctrl+L 走同一逻辑。

## 变更
### Ctrl+U 改为复用 Ctrl+L 逻辑（`crates/tui/src/app_helpers.rs`）
- `pre_key_intercept` 的匹配条件由 `Char('l')` 扩为
  `Char('l') | Char('u')`（app_helpers.rs:60），二者共用同一处理块：
  退出 subagent 视图、折叠全部 thinking、清空输入框、光标归零。

### 移除 `apply_scroll` 中的 Ctrl+U 分支（`crates/tui/src/key_handler.rs`）
- 删除 `apply_scroll` 开头的 Ctrl+U（`scroll - 10`）分支；该函数现仅处理
  PageUp / PageDown。
- 同步删除已失效的单元测试 `apply_scroll_ctrl_u`。
- 更新相关文档注释（dispatch 点与函数 doc）。

### 帮助文本合并（`crates/tui/src/keybind.rs`）
- 原 `Ctrl+U scroll up` 与 `Ctrl+L ...` 两行合并为单行
  `Ctrl+U / Ctrl+L  exit subagent view (if focused) / collapse all thinking / clear input`。

### 既有 changelog 修正
- `tui-disable-input-subagent-view.md` 原描述 `apply_scroll` 处理 Ctrl+U 滚动并
  列有 `apply_scroll_ctrl_u` 测试；本次随实现移除后已同步更正该条目，避免留下
  指向已删除代码的失真记忆（repair-on-touch）。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| Ctrl+U 被消费并清空输入、归零光标，且与 Ctrl+L 结果一致 | `ctrl_u_matches_ctrl_l_clears_input` | `tui/src/app_helpers_tests.rs`（新增） |

- 全量回归：`cargo test --workspace` → 896 passed; 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 行数：app_helpers.rs 639 / key_handler.rs 502 / keybind.rs 24（均 ≤ 800）

## Impact Surface
- 仅影响 TUI 按键处理与帮助文本；不触碰 session runner / store / LLM / core / web / cli。
- Ctrl+U 失去原「向上滚动」能力（滚动改由 PageUp 承担），其余按键行为不变。

## Related Docs
- [agents/tui](../../agents/tui/index.md)
- [既有 changelog：subagent 视图禁用输入](tui-disable-input-subagent-view.md)
