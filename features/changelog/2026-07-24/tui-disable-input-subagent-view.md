Commit: (working-tree, pre-initial-commit)

# feat(tui): 进入 subagent 视图时禁用文本输入

## 背景

TUI 在主会话中点击某个 subagent 条目后会进入 subagent-focus 视图（`subagent_focus`
为 `Some`），用于查看该子代理的完整交互流。但此前进入该视图后，**输入框仍然可输入、
可提交**：用户随手敲的字符会进入主会话输入草稿，Enter 甚至会把文本作为 steer/queue
提交到正在运行的 turn——这与「只读查看」的意图相悖。

此外，body 滚动键（PageUp / PageDown / Ctrl+U）此前内联在 `handle_key` 的输入可用
分支里，若简单地在 subagent 视图 early-return，会连带把滚动也禁掉，导致查看长 transcript
时无法翻页。

需求：**进入 subagent 视图时禁用文本输入与提交，但保留滚动与少量全局键**（退出 /
帮助），并给输入框一个清晰的「已禁用」视觉提示。

## 变更

### `handle_key` 新增 `input_disabled` 参数（`crates/tui/src/key_handler.rs`）

- 抽取 `pub(crate) fn apply_scroll(k, scroll, follow) -> bool`：统一处理
  PageUp（`scroll - 20`）/ PageDown（`follow = true`），
  返回是否消费了该键。该函数在输入可用与禁用两种状态下都被调用，保证滚动始终可用。
- 原内联的 PageUp / PageDown 分支删除，改为调用 `apply_scroll`。
- 新增 early-return 块：当 `input_disabled == true` 时，
  - 先执行 `apply_scroll`（滚动可用）；
  - 仅保留全局键：Ctrl+D（Quit）、Ctrl+H（切换 Help）；
  - 其余按键一律返回 `KeyAction::None`（字符、Enter、历史、提交等全部静默忽略）。

### `render` / `render_composer` 新增 `input_disabled` 参数（`crates/tui/src/render.rs`）

- `render_composer` 在 `disabled == true` 时渲染一个 `DarkGray + DIM` 风格的边框块，
  内容为提示行 `❯ ↞ esc / Ctrl+L to return`（告知用户如何退出），并直接 return，
  不渲染实际输入文本。
- `render` 在 `input_disabled` 时跳过 `place_cursor`——光标在禁用视图中隐藏。

### 接线（`crates/tui/src/app.rs`）

- 两处 `handle_key` 调用点与一处 `render` 调用点均传入 `subagent_focus.is_some()`，
  即当处于 subagent-focus 视图时 `input_disabled = true`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| `apply_scroll` PageUp 减少 scroll、关闭 follow | `apply_scroll_page_up` | `tui/src/key_handler.rs` |
| `apply_scroll` PageDown 打开 follow | `apply_scroll_page_down` | `tui/src/key_handler.rs` |
| `apply_scroll` 普通字符不被消费 | `apply_scroll_char_not_consumed` | `tui/src/key_handler.rs` |
| 禁用态字符键被忽略（输入框保持空） | `handle_key_disabled_blocks_char` | `tui/src/key_handler.rs` |
| 禁用态 Enter 不提交（返回 None） | `handle_key_disabled_blocks_enter` | `tui/src/key_handler.rs` |
| 禁用态 PageUp 仍可滚动 | `handle_key_disabled_allows_scroll` | `tui/src/key_handler.rs` |
| 禁用态 Ctrl+D 仍可退出 | `handle_key_disabled_allows_quit` | `tui/src/key_handler.rs` |

## Gate

| 项 | 结果 |
|----|------|
| `cargo test -p opencoder-tui --lib` | 349 passed / 0 failed / 0 ignored |
| `cargo clippy --workspace --all-targets -- -D warnings` | 零警告 |
| `cargo build --workspace` | 零错误 |
| 行数合规 | app.rs 793 / key_handler.rs 502 / render.rs 719（均 < 800） |

## Impact Surface

- 仅影响 TUI 渲染与按键处理；不触碰 session runner / store / LLM / core / web / cli。
- `input_disabled == false`（正常会话视图）时行为与变更前完全一致：滚动语义不变，
  文本输入与提交路径不受影响。

## 备注

- `cargo test --workspace` 存在一个与本特性无关的既有 flaky 测试
  `sys_tokens_counts_system_prompt`（`app_tests.rs`），它使用
  `std::env::temp_dir()` 作为工作目录，并发多 crate 测试二进制时会因文件系统争用而
  间歇性失败；隔离运行 `opencode-tui --lib` 时稳定 343/0。建议后续单独修复（改用
  `tempfile::tempdir()`），不在本特性范围内。

## Related Docs

- [agents/tui](../../agents/tui/index.md)
