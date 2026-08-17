Commit: (working-tree, post-1c0f426)

# Ctrl+G copy 模式三视图整屏零边框（composer + notepad 净化视图）

## Context

1c0b35e 已完成 copy 模式 body 净化视图（去边框/滚动条/[turn cost] 行，`clean_text` 行级净化），但 composer（含 plan/annotation 全屏编辑器）与 `/notepad` IDE 视图仍以装饰态渲染——终端原生选择在这两处取到的是带边框/行号槽/提示符的脏文本。本轮补齐三视图整屏零边框的最后两块，并压制 copy 激活期间的硬件光标（caret 块干扰原生选择）。

## Change Summary

- `crates/tui/src/copy_mode.rs`（387→481 行，≤800）：新增两个净化渲染器——
  - `render_composer_clean(f, area, input)`：composer 输入纯文本整屏渲染，无 block/边框、无 `❯ ` 提示符、无续行 padding、无附件 badge；复用 `composer::wrap_rows`（prompt_w=0）保证换行模型与装饰态 composer 完全一致。
  - `render_notepad_clean(f, area, view)`：notepad 编辑器缓冲的视觉行整屏渲染，无文件树面板、无边框、无行号 gutter、无 cmdline 行；行文本来自新函数 `notepad::editor::row_texts`（同一 `EditorLayout` 换行模型，装饰渲染器与净化视图永不发散）。
- `crates/tui/src/notepad/editor.rs`（新增 `row_texts` + 测试；655→691 行）：`EditorLayout::rows()` 的纯文本投影，纯函数、无编辑器状态。
- `crates/tui/src/render.rs`（799→798 行，≤800 红线守住）：
  - notepad 分支在 `copy_mode` 时改走 `render_notepad_clean`（渲染层守卫，与 `render_body` 的 copy_mode 早退同构）。
  - `render_composer` 新增 `copy_mode` 参数，置位时早退进 `render_composer_clean`。
  - `set_cursor_position` 守卫追加 `&& !copy_mode`：copy 激活期间不落硬件光标（caret 块与终端原生选择互抢）；弹窗自有光标路径不受影响。
  - `mode_flash_bg` 迁移至 `crates/tui/src/theme.rs`（582→595 行，与 `agent_chip_fg` 同居主题模块）；render.rs 侧的纯代理 `agent_chip_fg` 一并内联为 `theme::agent_chip_fg` 直调（task.rs 同步）。
- **与并发迭代的共存语义**：本轮不触碰 `copy_mode::handle_key` 的 overlay-yield 守卫（plan-edit/notepad 打开时 copy toggle 让位、按键透传给 overlay——并发未提交迭代已落地的设计）。两个净化渲染器是**渲染层防御性分层**：若 overlay 语义未来放宽，净化视图自动跟随；当前正常聊天视图（body+composer）是主要可达路径，notepad/plan-edit 净化路径由单测直接钉死。

## Validation（当次实跑）

- `cargo test -p opencoder-tui --lib`：**1388 passed / 0 failed**（含本轮新增 5 测试）。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告（Finished dev profile）。
- `cargo test --workspace --no-fail-fast`：**2855 passed / 0 failed**（EXIT=0；工作树同时含并发未提交迭代的全部改动，基线 2804 + 并发迭代新增 + 本轮 5）。

## 测试覆盖表

| 测试 | 层 | 覆盖点 |
|---|---|---|
| `copy_mode.rs::render_composer_clean_shows_text_without_chrome` | unit（TestBackend e2e） | 净化 composer：文本 flush 左 0 列、无 `❯`、无边框字符 |
| `copy_mode.rs::render_notepad_clean_shows_file_text_without_chrome` | unit（TestBackend e2e） | 净化 notepad：文件文本 flush 左 0 列（非行号 gutter）、无边框/竖线装饰 |
| `render_tests/composer.rs::composer_copy_mode_param_early_exits_to_clean_view` | integration | `render_composer` copy_mode 参数早退：装饰参数（PLAN 标签/标题/badge）全部不生效 |
| `render_tests/cursor_popup.rs::copy_mode_suppresses_composer_hardware_cursor` | integration（全量 render()） | copy 激活期间不落硬件光标（对照非 copy 态正常落点） |
| `notepad/editor.rs::row_texts_round_trips_and_wraps_in_order` | unit | 行文本投影 round-trip 不丢字符、窄宽换行有序、空缓冲单空行 |

无删测试 / 无 `#[ignore]` / 无弱断言；render.rs 798 ≤800、copy_mode.rs 481 ≤800、theme.rs 595 ≤800、notepad/editor.rs 691 ≤800。
