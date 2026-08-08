# feat(tui): subagent ctx-switch 时状态栏 mode chip 显示 subagent 类型

## 背景

当 TUI 进入 subagent 焦点视图（ctx-switch，Ctrl+L 进入 / Esc 返回）时，底部状态栏的
mode chip（`● [act]` 绿色圆点 + 方括号标签）始终显示父级 agent 名（`act`/`plan`），
而非当前正在查看的 subagent 类型（`explore`/`build`）。用户无法从状态栏一眼看出当前
视图切换到了哪个子代理。

## 变更

引入 `display_mode` 字段，从 `compute_display` 一路传递到 `render_status`，替代原先
直接读取的 `&chat.agent`：

- **`app_loop.rs`**：`DisplayState` 新增 `display_mode: String`。`compute_display` 中，
  subagent 焦点分支设 `display_mode = kind.clone()`（如 `"explore"`），其余分支（顶层 /
  fallback）设 `display_mode = agent_name.clone()`（如 `"act"`）。
- **`app.rs`**：解构 `display_mode`，作为 `render_frame` 的末位参数传入。
- **`frame.rs`**：`render_frame` 末位加 `display_mode: &str`，转发到 `render`。
- **`render.rs`**：`render` 末位加 `display_mode: &str`；`render_status` 调用从
  `&chat.agent` 改为 `display_mode`（`render_status` 本身未改，已用 `mode: &str` 参数
  驱动 `theme::agent_chip_fg(mode)` 着色）。

`display_mode` 是 display-only `String`，不写入 child view 的 `.agent` 字段，因此不影响
`sys_tokens_for` / `chat.rs` 中 `self.agent == "plan"` 的读取逻辑。

## 测试清单（3 项）

| 功能 | 测试名 | 文件 | 断言 |
| --- | --- | --- | --- |
| 顶层 mode chip 显示 act | `compute_display_title_uses_workdir_style_for_model_and_effort` | `app_loop_tests/display_title_tests.rs` | `ds.display_mode == "act"` |
| 空努力字段不残留分隔符 | `compute_display_title_omits_blank_effort` | `app_loop_tests/display_title_tests.rs` | （既有，display_mode 无副作用） |
| subagent 焦点 mode chip 显示 explore | `compute_display_subagent_title_keeps_navigation` | `app_loop_tests/display_title_tests.rs` | `ds.display_mode == "explore"` |

5 处直接 `render()` 调用补 `"act"` 末位参数（`render_clear_tests.rs` ×1、
`render_tests/chips.rs` ×2、`render_tests/arrow_click.rs` ×2）。

## Gate

- `cargo test -p opencoder-tui` → **1088 passed / 0 failed**（EXIT=0）
- `cargo test --workspace` → **2093 passed / 0 failed**（EXIT=0）
- `cargo clippy -p opencoder-tui --all-targets -- -D warnings` → **0 warnings / 0 errors**
- 行数：`app.rs` 800 / `app_loop.rs` 786 / `render.rs` 788 / `frame.rs` 267，均 ≤ 800
