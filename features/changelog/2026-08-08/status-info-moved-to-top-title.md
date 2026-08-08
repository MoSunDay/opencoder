Commit: (working-tree, pre-initial-commit)

# feat(tui): 底部状态栏 model/[mode] 上移合并到顶部 body 标题

## 背景

底部状态栏原先依次渲染 `model · [agent] · ctx进度 · 耗时 · spinner`，信息密度过高；
顶部 body 标题只显示 workdir 路径。用户要求把 模型名、思考深度（reasoning_effort）、
模式（`[act]`/`[plan]`）上移合并到顶部标题，顺序为 **workdir / 模式 / 模型名 / 思考深度**
（如 `/root/opencoder · [act] · glm-5.2 · high`，effort 未设/空白时省略末尾）。
底部状态栏只保留 ctx 进度条/百分比、任务耗时、spinner/status。

## 变更

- **`app_loop.rs` `compute_display`**：顶层分支的 `display_title` 由纯字符串 workdir
  改为带样式的 `Line<'static>` 组合 `{workdir} · [{mode}] · {model}`（+ ` · {effort}`），
  分段颜色沿用原状态栏（mode 用 `theme::agent_chip_fg`、model/effort 用 `theme::text()`）；
  裸 model id 仍经 `config.model_id()` 去 provider 前缀。移除 `DisplayState.status_model`
  与 `display_status_agent` 字段。subagent 聚焦标题（`← [Ctrl+L] back | ⤷sub …`）不变。
- **`render.rs`**：`render_status` 删除 `model`/`agent` 参数及对应 spans（保留 ctx
  meter/百分比/耗时/spinner）；`render()` 同步删除 `model`、`agent` 参数，`title` 类型
  改为 `&Line<'static>`；`render_body` 用新增 `theme::rounded_block_line` 渲染多段样式标题。
- **`theme.rs`**：新增 `rounded_block_line(&Line<'static>)`（带首尾空格 padding，
  与 `rounded_block(&str)` 一致）。
- **`frame.rs` / `app.rs`**：`render_frame` 删除 `agent`/`model` 参数；调用点删除
  `display_status_agent`/`status_model` 的解构与传参。
- 样式与颜色保持不变：`[mode]` 徽标色、model/effort 文本色、`·` 分隔符均沿用原底部状态栏。
- `agent_chip_fg` 仍被 `/task` picker 使用，保留。

## 测试清单（crates/tui，全部为 unit，TestBackend / 纯函数）

| 行为 | 测试名 | 层 |
| --- | --- | --- |
| 标题 = workdir · [mode] · 裸 model id（去 provider 前缀） | `compute_display_title_strips_provider_prefix` | unit |
| 顺序 workdir → mode → model → effort，effort 徽标拼接 | `compute_display_title_with_effort_strips_prefix` | unit |
| 空白 reasoning_effort 省略末尾徽标 | `compute_display_title_omits_blank_effort` | unit |
| body 顶部标题行含完整组合序列 | `body_title_row_shows_full_top_composition` | unit(render) |
| rounded_block_line 首尾空格 padding 与 rounded_block 一致（直接单测） | `rounded_block_line_pads_title_like_rounded_block` | unit(theme) |
| 底部状态栏不再含 model / [mode]（已上移），仍含 ctx、无品牌 | `status_bar_omits_branding_and_top_moved_info` | unit(render) |
| 既有 status_bar/status_ctx 各用例随 `render_status` 签名更新后全绿 | `status_bar_*` / `status_ctx_*`（8 例） | unit(render) |

## Gate

- 全量回归：`cargo test --workspace` → **2024 passed / 0 failed / 0 ignored**（当次实跑，含补测 `rounded_block_line_pads_title_like_rounded_block`）

- 回归基线归因：基线 b1e1e7f 记录 2023 passed；当次树 2023 = 基线 2023 + 本 scope 新增 9
  测试 − 9（并行 agent 在 0cff6e0 同树删除 help/short_key 旧功能测试并重构旧 task timer
  测试，删改均由其 changelog `tui-ctrl-h-keymap-menu-migration.md` 文档化）——净计数持平
  基线而非 +9，非本 scope 静默回归。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：app_loop.rs 762 / render.rs 792 / app.rs 794（均 < 800），theme.rs 455（含
  `rounded_block_line` 实现 + 直接单测；该单测由本次整改补入）。app_loop_tests/mod.rs
  已由后续提交 451738f 拆分为 display_title_tests.rs（789 + 102 行，均达标）
