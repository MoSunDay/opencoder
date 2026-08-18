Commit: (working-tree, post-860831d)

# Ctrl+G copy 模式全局 toggle 修复：overlay 打开不再是死键 + notepad 净化视图补 COPY MODE chip

## Context

2026-08-17 提交 7e7da28 第 #4 项为 `copy_mode::handle_key` 引入 overlay-yield 守卫：plan-edit/notepad overlay 打开时直接 `return false`（toggle 与吞键全部让位、按键透传给 overlay）。该守卫使 overlay 编辑期间 Ctrl+G 成为**死键**——用户无法进入 copy 模式做终端原生选择，与 2026-08-17 三视图整屏净化（`render_composer_clean`/`render_notepad_clean` 已就位）的初衷相悖：净化渲染层本就是为 overlay 打开时进 copy 模式准备的可达路径，守卫却把入口焊死。且 notepad 全屏分支在 render.rs 中早退于状态 chip 渲染之前，即便强行进入 copy 模式也是"隐形模式"（无任何可见反馈）。

## Change Summary

- `crates/tui/src/copy_mode/mod.rs`（490→539 行，≤800）：
  - `handle_key` 删除 `overlay_active` 参数与提前返回：toggle 恢复**全局语义**——plan-edit/notepad overlay 打开时 Ctrl+G 照常进出 copy 模式；活跃时吞掉所有键，Ctrl+G/Esc 退出，退出后 overlay 原样保留（分层模态：首个 Esc 先退 copy 模式，下一个 Esc 才关 overlay）。纯函数 `next_state` 契约不动。
  - `render_notepad_clean` 末行补 "COPY MODE: Ctrl+G/Esc" chip（新私有 helper `render_copy_chip`，最小复刻 render.rs 私有 `render_status_chip` 的画法：Clear + 黑字/警示底/加粗、右对齐留 1 列边距；render.rs 零改动）。chip 文本纯 ASCII，不引入 `┌ └ │ ─` 制表符，不破坏"全屏无 chrome"断言；钉在末行，文件文本仍 flush 左 0 列可整屏选择。
- `crates/tui/src/app.rs`（799→799 行，≤800）：:300 调用点去掉第 4 个实参 `plan_edit.is_some() || notepad.is_some()`。
- 记忆 repair-on-touch：`agents/tui/index.md`（copy_mode 条目）与 `features/index.md`（信息区文本选择条目）的「overlay 打开时 toggle 让位透传」表述改为新语义（全局 toggle + notepad 净化视图自带 chip + 分层模态 Esc）。

## 影响（语义变化）

notepad/plan-edit overlay 打开时的按键行为变化：Ctrl+G 由"透传死键"变为"进 copy 模式"；copy 模式活跃期间 overlay 收不到任何键（含 Esc——首个 Esc 先退 copy 模式，再按 Esc 才关 overlay），退出 copy 模式后 overlay 状态原样保留。normal 聊天视图行为不变。

## Validation（当次实跑）

- `cargo test -p opencoder-tui --lib`：**1426 passed / 0 failed**（基线 1426；删 3 个旧语义测试、新增 3 个新语义/渲染测试，净 0）。
- `cargo clippy -p opencoder-tui --all-targets -- -D warnings`：零警告（Finished dev profile）。
- `wc -l`：copy_mode/mod.rs 539 ≤800、app.rs 799 ≤800。

## 测试覆盖表

| 测试 | 层 | 覆盖点 |
|---|---|---|
| `copy_mode::tests::toggle_fires_even_with_overlay_open`（新，替代 `overlay_active_ignores_toggle_key`/`overlay_inactive_toggles_normally`） | unit | 新签名下 Ctrl+G 使 inactive→active 且返回 true（overlay 打开也照进），再次 Ctrl+G 退出 |
| `copy_mode::tests::active_swallows_keys_and_esc_exits`（新，替代 `overlay_active_does_not_swallow_when_copy_mode_active`） | unit | 活跃时普通键被吞返回 true 且状态保持；Esc 返回 true 且翻 false；退出后恢复透传 |
| `copy_mode::tests::render_notepad_clean_shows_copy_mode_chip`（新） | unit（TestBackend e2e） | 净化 notepad 末行含 "COPY MODE: Ctrl+G/Esc" chip；文件文本仍 flush 左 0 列；chip 不引入制表符 |
| `copy_mode::tests::render_notepad_clean_shows_file_text_without_chrome`（既有，保留通过） | unit | chip 加入后旧断言（rows[0]=alpha-line、无 `┌ └ │`）不回归 |
| `copy_mode::tests::next_state` 四件套（既有） | unit | 纯函数契约未动：toggle 翻转/活跃吞键/Esc 退出/非 toggle 透传 |
| `render_tests::composer::full_frame_annotation_editor_copy_mode_hides_border`（新，post-hoc） | integration（TestBackend 全帧） | `/annotation` 编辑器 overlay + copy 模式全帧端到端：编辑文本 col 0 裸排、无 border/prompt glyph/编辑器标题、COPY MODE chip 仍可见——补上 `composer_copy_mode_param_early_exits_to_clean_view` 单元缝隙未覆盖的 `render` 集成面 |

无 `#[ignore]` / 无弱断言；删测试均有等语义新测试接替（旧语义本身即被修复的 bug）。
