Commit: cff0b2751ba7893d0afde18b62455b3ac000a42c

# 修复：copy 模式（Ctrl+G）终端原生复制在软换行处插入多余换行

## 背景与根因

Copy 模式（Ctrl+G）的定位是「终端原生选择 + 复制」，但长行在软换行边界被复制成多行。根因在 ratatui 的 `CrosstermBackend`：`draw` 对每个可视行都发射 `MoveTo`（CSI `y;x H`）。终端据此把每个可视行当成硬行（hard row），DECAWM 自动换行状态被 MoveTo 打断，原生复制即按硬行插入换行。

## 机制

新增 wrap-aware 后端（仅 copy 模式启用），利用终端自身 DECAWM auto-wrap 标记换行行：

- `WrapPlan`（`Rc<RefCell>`，backend 与 renderer 共享）是本帧唯一事实源：`active` 镜像 `copy_mode`；`soft` 软换行标志每帧由 copy 模式的渲染器重新填充；`render()` 在帧首清空 `soft` 并写入 `term_width`。
- 后端软边界判定（全条件满足才跳过 MoveTo，让 DECAWM 自动换行标记该行）：plan.active、width>0、x==0、y>0、上一格在 `(width-1, y-1)` 打印过非空符号、`plan.soft[y]`、样式未变（边界处有 SGR 序列 → 保守走 MoveTo）。
- 硬换行（真实换行，含恰好整倍宽度的逻辑行）始终保留 MoveTo；内容之后的空行视为硬行；非 copy 模式逐字节委托给原后端，输出与改造前完全一致。
- 渲染器仅在 `wp.term_width == area.width` 时填 flag（宽度不匹配守卫），并使用 `area.y`/`text_area.y` 偏移（`reserve_chip_row` 时文本可能从 `area.y+1` 开始）。

## 文件

- `crates/tui/src/copy_wrap.rs`（新增，320 行）：`WrapPlan`、三个纯函数（`soft_flags_from_cum_rows` / `soft_flags_from_wrap_rows` / `soft_flags_from_row_texts`）、`WrapAwareBackend`（委托非 draw 方法；draw 镜像 ratatui 的 `ModifierDiff` 与 underline-color diff，保证字节级一致）、`frame_plan`。
- `crates/tui/src/copy_wrap_tests.rs` / `copy_wrap_fill_tests.rs`（新增，352+193 行）：字节级后端测试 + 纯函数测试（前者），经 3 个 clean 渲染器的 plan 填充集成测试（后者）。
- `crates/tui/src/app_bootstrap.rs`：生产 backend 换为 `WrapAwareBackend`，与 renderer 共享 plan。
- `crates/tui/src/render.rs`（798 行）：`Term` 别名、帧首 plan 装配（downcast）、参数透传。
- `crates/tui/src/copy_mode/mod.rs`（766 行）：transcript / composer / notepad 三个 clean 渲染器填 soft flag。
- `crates/tui/src/render_viewport.rs`：`CleanModel::cum_rows()` 访问器。
- `crates/tui/src/lib.rs`：`pub mod copy_wrap;`。

## Validation

- TUI 全量：`cargo test -p opencoder-tui --lib` → 1530 passed / 0 failed。
- 全量回归：`cargo test --workspace` → 3280 passed / 0 failed（含 `client_server_flag_matrix_smoke`；负载高峰下偶发 `server never became reachable` 超时，孤立重跑 1.11s 通过，属环境性抖动）。
- Lint gate：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告。
- 构建 gate：`cargo build --workspace` → 零错误；`cargo fmt --all -- --check`、`git diff --check` 通过。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 软边界跳过 MoveTo（字节级） | `soft_boundary_skips_moveto_and_relies_on_terminal_wrap` | `tui/src/copy_wrap_tests.rs` |
| 硬边界保留 MoveTo 且字节与 plain 后端一致 | `hard_boundary_keeps_moveto_identical_to_plain` | 同上 |
| 恰好整倍宽度行：行内软、下一行硬 | `exact_width_line_joins_inside_but_stays_hard_at_next_line` | 同上 |
| 非 copy 模式逐字节等同 plain | `inactive_output_matches_plain_backend_byte_for_byte` | 同上 |
| 边界 SGR 样式变化 → 保守 MoveTo | `style_change_at_boundary_falls_back_to_moveto` | 同上 |
| 空符号（宽字符续格）不触发软边界 | `zero_width_and_missing_soft_flags_are_hard` | 同上 |
| 非末列跳跃始终硬 | `jump_not_from_last_column_is_hard_even_when_soft` | 同上 |
| cum_rows 软标志推导 | `cum_rows_flags` | 同上 |
| wrap_rows / row_texts 软标志推导 | `wrap_rows_flags` / `row_texts_flags` | 同上 |
| set_soft 拼接增长 | `set_soft_splices_and_grows` | 同上 |
| clean 渲染器填 flag（transcript 折行/短行、composer、notepad、area 偏移、宽度不匹配） | `render_clean_fills_soft_for_wrapped_transcript` / `render_clean_fills_hard_for_short_lines` / `render_composer_clean_fills_soft` / `render_notepad_clean_fills_soft` / `render_clean_respects_area_offset_and_width_mismatch` | 同上 |
| 既有渲染测试（render_body / render_composer 新参数） | 全部既有 `render_*` 用例 | `tui/src/render_tests/*` |

## 兼容性与边界

- 无数据库 schema、配置项、环境变量或公共 API 变化。
- 测试环境（`TestBackend`）downcast 返回 `None`，plan 为空操作，渲染字节不变。
- 手动真终端验证项（Kitty/WezTerm/alacritty/tmux）：长行复制无换行、真实换行保留、整倍宽行保留换行、非 copy 模式逐字节一致。
