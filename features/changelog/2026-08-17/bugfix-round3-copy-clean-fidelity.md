# Bug 扫除第三轮：copy 净化视图保真重构（独立可回滚）

`copy_mode.rs` 拆分为 `copy_mode/{mod,clean}.rs`，净化逻辑从文本启发式改为 **span 结构化**，滚动几何改按**净化后行集**计算。范围延续前两轮红线：仅 5 个 tui 文件，skill 区与并行 WIP（question_menu*、agents/tui/index.md）未触碰。

## 用户可见变更
- **copy 模式选择不再误杀内容**：代码块内的 `---`（YAML frontmatter）、`└───`、`─×19`、`┌ x ` 等字符行此前可能被当成装饰删除（`starts_with('┌'/'└')`、`is_separator` 启发式），现在只有渲染器产出的精确装饰形状被删。
- **gutter 剥离精确化**：正文自带行首空格不再被"先猜 4 后猜 2"削错槽位（gutter 是独立 span，按结构剥）；plan 正文 2 空格、user/assistant 4 空格槽位精确。
- **滚动保真**：装饰行满屏不再留底部空白带；首可见行不再被整行吞掉（旧 `top_skip=0` 补丁删除，窗口数学直接在净化行集上）。
- tool/thinking 正文与 image 兜底行的前导空格保留（原被放宽剥离）。

## 变更文件
- `tui/src/markdown.rs`（414 行）：导出 6 个形状常量（`CODE_TOP_PREFIX`/`CODE_BOTTOM`(└+19─)/`RULE_LINE`(─×19)/`CODE_ROW_PREFIX`/`CODE_ROW_EMPTY`/`QUOTE_PREFIX`），渲染处改用（输出逐字节不变）。
- `tui/src/chat.rs`（727 行）：导出 `ROLE_USER_HEADER`/`ROLE_SAY_HEADER`/`PLAN_HEADER` + 3 处接线（仅此）。
- `tui/src/copy_mode/clean.rs`（新，392 行 ≤400）：`LineKind`/`classify`（span 结构判定 + 精确槽宽）/`clean_line`。
- `tui/src/copy_mode/mod.rs`（490 行）：原 `copy_mode.rs` 迁入；`render_clean` 改为 CleanModel 驱动。
- `tui/src/render_viewport.rs`（443 行）：`CleanModel{texts,cum_rows,total_rows,width}` + `ViewportCache::cleaned()` 懒构建（宽度失效重建）。
- `render.rs` 零改动（模块路径不变）。

形状常量为唯一真源：判定处与渲染处引用同一常量，形状漂移即编译期可见。

## 语义变化清单
收紧：仅以 `┌`/`└` 开头的正文行不再误删（精确单 span 帧形状 + gutter 后同构识别）；`is_separator` 废除（裸 `---`/`────` 不再当分隔符，代码块内 `---` 因带 `│ ` 前置 span 保留）；gutter 按独立 span 精确剥。放宽：tool/thinking 正文与 image 兜底行保留前导空格；代码行内容不再二次剥 `▎ `（quote 仅在 Text 行首 span 内剥）。几何：clamp/窗口/top_skip 全在净化行集上计算。

## 测试清单（rules/01）
- clean.rs 表驱动 7 组：`decoration_shapes_are_dropped`(12)、`decoration_glyphs_inside_code_rows_survive`(6)、`slots_are_stripped_structurally`(7)、`slotless_rows_pass_through_untouched`(9)、`empty_rows_and_padding`(5)、`classify_reports_kinds_and_exact_slots`(13)、`plain_text_concatenates_spans`
- mod.rs 渲染级：新增 `render_clean_keeps_separator_like_code_rows`、`render_clean_keeps_text_leading_spaces_beyond_gutter`、`render_clean_no_blank_band_under_trailing_decoration`；原 code-frame/composer/notepad 渲染与 9 个键处理/overlay 测试迁移
- render_viewport.rs：`cleaned_total_counts_only_kept_rows`、`cleaned_visible_window_maps_wrapped_rows`、`cleaned_top_skip_takes_first_visible_line_own_offset`、`cleaned_trailing_decoration_leaves_no_blank_band`、`cleaned_is_cached_and_rebuilt_on_width_change`、`cleaned_empty_cache_is_empty`

## 回归 gate（rules/02）
`cargo test --workspace --no-fail-fast`：**2915 passed / 0 failed** ✓
`cargo clippy --workspace --all-targets -- -D warnings`：零警告 ✓
`cargo build --workspace`：干净 ✓；改动文件 `rustfmt --check` 干净

## Impact Surface
- 用户：copy 模式（Ctrl+G）下的终端原生选择得到干净且完整的文本；滚动行为不变但无空白带/吞行。
- 不影响：正常渲染路径（`ViewportCache` 主路径、hits、app.rs 键处理）、skill 区、todos/core/session。
