Commit: 3865e73
# anno/plan 编辑器 copy 模式：首行预留为 COPY MODE chip 专用空行，长行行尾不再被覆盖

## Context

用户报障：annotation 编辑器 copy 模式下，"COPY MODE: Ctrl+G/Esc" chip 由 `render_status_chip` 钉在 composer 区**首行右侧**（render.rs:762），而净化视图 `render_composer_clean` 的文本也从首行开始满宽裸排（render.rs:639）——两者同行冲突：触到右边缘的长注释行行尾被 chip 的 Clear+底色块覆盖，无法完整选择。notepad 净化视图早有先例（chip 钉**末行**、文件文本从首行开始，`render_copy_chip`），但 anno/plan overlay 走 composer 分支，chip 在首行。plan 编辑器（`edit plan`）与 anno 同一代码路径，同缺陷同修。

## Change

- `crates/tui/src/copy_mode/mod.rs`：`render_composer_clean` 新增 `reserve_chip_row: bool` 参数——true 时文本渲染进 `Rect{y: area.y+1, height: area.height-1}`（宽度不变，wrap 模型不动），首行留空给 chip；doc 注释说明两种调用方的分工。
- `crates/tui/src/render.rs:639`：调用点原地传 `plan_mode.is_some()`（anno/plan overlay → 预留；普通 composer copy 模式不变——其输入行通常短、正文转录才是复制目标，且 composer 区矮，空行会挤掉输入行）。render.rs 798 行不变（原地实参改写，零增行）。

## 语义

anno/plan 编辑器 copy 模式：row0 = chip 专用空行，row1 起为满宽裸排注释文本；终端原生选择可完整覆盖长行行尾。普通 composer copy 模式渲染不变。

## 测试清单

| 测试 | 文件 | 覆盖点 |
|---|---|---|
| `render_composer_clean_reserved_row0_stays_blank_for_chip` | `crates/tui/src/copy_mode/mod.rs` | reserve=true 时 row0 全空、row1/row2 起 flush 文本、无 chrome |
| `render_composer_clean_shows_text_without_chrome` | `crates/tui/src/copy_mode/mod.rs` | 既有用例补 `false` 实参：普通 composer 语义不变（row0 起文本） |
| `composer_copy_mode_param_early_exits_to_clean_view` | `crates/tui/src/render_tests/composer.rs` | seam 级：plan_mode=Some 时 row0 预留、row1 起文本 |
| `full_frame_annotation_editor_copy_mode_hides_border` | `crates/tui/src/render_tests/composer.rs` | 全帧：row0 仅含 chip 不含注释文本，row1 以注释文本开头 |

## Validation

- `cargo test -p opencoder-tui --lib`：**1433 passed / 0 failed**（含 4 条上述用例）。
- `cargo clippy -p opencoder-tui --lib -- -D warnings -A dead_code`：零警告（`worker.rs::handoff_run_prompt` dead-code 为并行会话在途 WIP——其调用点在 `app_helpers.rs` 未提交改动中被删——非本任务文件，未触碰）。
- `wc -l`：render.rs 798 ≤800（零增行）、copy_mode/mod.rs 586 ≤800、render_tests/composer.rs 289 ≤800。
- pty 端到端（装后实测）：anno 编辑器输入触右边缘长行 → C-g → 首行仅 chip、文本行完整可整行选择，`:wq` 正常退出。
