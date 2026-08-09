Commit: (working-tree, uncommitted)

# feat(tui): /requirement 编辑器顶边镜像 body 顶部标题（workdir · model · effort）

## 背景

`/requirement`（plan_mode 激活、`edit_title == "edit requirement"`）编辑器原先只在其顶部圆角边框
左侧显示 ` edit requirement ` 标签，边框为 requirement 强调色（绿）。而 body 区域顶部已通过
`status-info-moved-to-top-title.md` 渲染了 `workdir · [mode] · model · effort` 组合标题。本次让
`/requirement` 编辑器在**同一条顶边**上、右侧再镜像出 body 的 `workdir · model · effort` 标题，
同样染绿色（`theme::ok_color()`），与左侧 ` edit requirement ` 标签并列。`/plan` 编辑器保持不变
（`edit_title == None` → ` edit plan `、warn 色、不显示 info 标题）。

## 变更

- **`theme.rs`**：新增 `pub fn title_spans_colored(&Line<'_>, fg: Color) -> Line<'static>`——为首尾各
  补一个 padding 空格（与 `rounded_block_line` 一致），并把每个 span 统一重染为 `fg`，用于右对齐的
  顶部边框标题。纯函数，无副作用。
- **`render.rs` `render_composer`**：新增参数 `top_title: &Line<'static>`。当 `edit_title == Some
  ("edit requirement")` 时，在原 `Block` 上追加一个**右对齐**标题
  `theme::title_spans_colored(top_title, theme::ok_color())`；`/plan`（`edit_title == None`）分支不追加。
  唯一生产调用点 `render()`（render.rs）已同步传入既有 body `title`。
- **`render_tests/composer.rs`**：新增 2 个渲染单测（见下表）；既有 `composer_renders_prompt_and_multiline_text`
  随新签名补传 `&Line::raw("ignored")`。
- **`render_tests/cursor.rs`**：既有 `composer_word_wrap_renders_...` 随新签名补参。
- 范围：纯表现层；私有 fn 签名变更，blast radius 仅 TUI crate，调用点全部一致更新。

## 测试清单（crates/tui，全部为 unit，TestBackend / 纯函数）

| 行为 | 测试名 | 层 |
| --- | --- | --- |
| `title_spans_colored`：3 内容 span + 首尾 padding = 5 span；拼接串与内容有序保留；每个 span `style.fg == Some(green)` | `title_spans_colored_pads_and_recolors_all_spans` | unit(theme) |
| `/requirement` 顶边含左侧 `edit requirement` 与右侧 model；按 char 偏移定位 model cell 断言像素级 `fg == Some(green)` | `requirement_editor_shows_green_top_title` | unit(render) |
| `/plan`（`edit_title == None`）顶边含 `edit plan` 且**不含** model（负向/边界） | `plan_editor_has_no_info_top_title` | unit(render) |

## Gate（隔离工作树实跑，仅含本变更）

本仓库主工作树当前存在并发的在途改动（`notepad/` 模块等，与本任务无关且阻断全树 gate），为剔除其干扰，
以下数字取自一个 `git worktree`（基于 HEAD `4671385`，仅 apply 了本变更的 4 个文件）：

- 回归基线（HEAD，无本变更）：`cargo test -p opencoder-tui --lib` → **1032 passed / 0 failed / 0 ignored**
- 全量回归（HEAD + 本变更）：`cargo test -p opencoder-tui --lib` → **1035 passed / 0 failed / 0 ignored**
  （Δ = +3，恰为本变更 3 个新测试；无删除/修改既有测试计数 → 无回归）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（EXIT=0）
- build：`cargo build --workspace` → 零错误（EXIT=0）
- 行数：render.rs 794（≤ 800）/ theme.rs 526（≤ 800）/ composer.rs 137（≤ 400，新文件）/ cursor.rs 122（≤ 800）
