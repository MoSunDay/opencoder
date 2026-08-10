Commit: 4ae5b50508e9d9016edeb45c61361240ecce1e37

# feat(tui): 顶部标题三项统一 workdir 无着色样式 + 底部状态栏去掉第二根进度条

## 背景

用户反馈两条 UI 精简意见：

1. 顶部 body 标题当前为 `workdir · model · thinking level`，其中 model name 和
   thinking level 用主题色着色、与 workdir 的纯文本样式不一致。要求把它们改成和
   workdir 一样——即标题仍按 `workdir · model · effort` 排列，但三项统一使用
   workdir 的无着色纯文本样式。
2. 底部状态栏有两根 10 段进度条（压缩阈值仪表 + 窗口用量仪表），要求去掉从左数第二根
   （窗口用量仪表）——ctx 文本已以数字报告窗口用量，视觉仪表保留压缩阈值一根即可。

## 变更

- **`crates/tui/src/app_display.rs`**：`compose_top_title(workdir, model_bare, effort)`
  中 model 与 effort 从主题色 `Span::styled` 改为 `Span::raw`（与 workdir 相同的
  无着色样式）；effort 为空/空白时省略；workdir/model/effort 均经
  `sanitize_single_line` 单行化。
- **`crates/tui/src/app_loop.rs`**：`compute_display` 顶层标题仍调
  `compose_top_title(workdir, config.model_id(), reasoning_effort)`；model 仍取
  `config.model_id()`（去 provider 前缀，如 `bigmodel/glm-5.2` → `glm-5.2`）。
- **`crates/tui/src/render.rs` `render_status`**：删除第二根窗口用量仪表
  （`win_meter`/`win_color`），保留压缩阈值仪表（`meter`，基于
  `compaction_threshold`）与 ctx 文本（`ctx N% (used/limit)`，分母仍为模型窗口）。

## 测试清单（crates/tui，全部为 unit，纯函数 / TestBackend）

| 行为 | 测试名 | 层 |
| --- | --- | --- |
| 标题中 model/effort 使用与 workdir 相同的无着色 raw span，且无 provider 前缀 | `compute_display_title_uses_workdir_style_for_model_and_effort` | unit(app_loop) |
| 空白 reasoning_effort 不参与标题 | `compute_display_title_omits_blank_effort` | unit(app_loop) |
| subagent 聚焦仍返回返回/导航标题（不受影响） | `compute_display_subagent_title_keeps_navigation` | unit(app_loop) |
| 标题纯函数：model/effort 用 workdir 样式（raw span） | `compose_title_uses_the_workdir_style_for_model_and_effort` | unit(app_display) |
| 空白 effort 省略 + 值单行化清洗 | `compose_title_omits_blank_effort_and_sanitizes_values` | unit(app_display) |
| body 标题行渲染完整 `workdir · model · effort` 组合 | `body_title_row_shows_full_top_composition` | unit(render) |
| 状态栏只剩一根仪表（used=0 全空 10 格） | `status_bar_has_single_meter_before_ctx` | unit(render) |
| 保留的仪表跟随压缩阈值（超阈值即满、无第二根） | `status_bar_single_dial_tracks_threshold_not_window` | unit(render) |

## Gate

- TUI 全量回归：`cargo test -p opencoder-tui --lib`
- TUI clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings`
- TUI build：`cargo build -p opencoder-tui`
- 行数：app_display.rs / app_loop.rs / render.rs 均 ≤ 800
