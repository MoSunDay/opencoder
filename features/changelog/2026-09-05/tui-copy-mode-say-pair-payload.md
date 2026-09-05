Commit: bf012b2

# TUI copy 模式：`Say(n steps)` 合并头保留 preview 载荷（Say 内容不再丢失）

## Context

Ctrl+G copy 模式以 `copy_mode/clean.rs` 的结构化净化渲染转录。f619bd5
引入合并头 `{❯|▸} Say(n step{s}): <preview>` 的**同一天**也让正文按
preview 去重（`merged_say_body` 三态 Full/Skip/Hidden：正文跳过首个
非空行；单行 Say 正文整块隐藏）。但 `is_say_pair_header` 仍把合并头归
`RoleHeader` 整行丢弃，注释依据「正文下方携带同一首行」已被去重推翻——
用户复报：**copy 模式下 Say 内容看不到**（单行 Say 完全消失，多行 Say
丢首行）；并要求已展开的 step thinking 等内容在 copy 模式同样可见。

## Change Summary

- **`LineKind::SayPairHeader`（clean.rs）**：合并头改归半装饰行——
  `classify_spans` 返回新 kind，`clean_line` 新增专用分支调用纯函数
  `say_pair_payload`：剥 label span 与尾部 `  ⠋ running ` spinner span
  （同 `count_row_label` 的 spinner 语法），保留 preview 载荷（trim，
  空 preview → `None` 整行丢弃，纯空白 Say 无可复制内容）。
- **展开内容可见性钉死**：展开阶梯的 step thinking 正文（Text 行剥
  gutter 保留）与展开 call 输出本就存活，新增 e2e 用例锁定该契约，
  防止后续净化规则回归误杀。
- 修正 `is_say_pair_header` 与模块头的过时注释；`copy_mode/mod.rs`
  模块文档补合并头半装饰语义。

## Impact Surface

- 仅 `crates/tui/src/copy_mode/{clean.rs, mod.rs}`；装饰态渲染/行数
  记账/点击 hit-rect 零改动（`flatten_with` 未动）。
- 测试：`chat_tests/say_pair.rs` 原 `copy_mode_strips_the_merged_header`
  （钉住错误丢弃）改写为 `copy_mode_keeps_the_merged_header_preview`；
  clean.rs 新增 `say_pair_headers_keep_preview_payload`（6 例表）+
  classify 表 2 例；新增 `copy_mode/clean_say_tests.rs`（≤400 行）4 例
  e2e（单行 Say 存活且 label 剥离、多行 Say 首行+其余行、流式 preview
  存活且 spinner 剥离、展开阶梯 thinking/call 输出/完整 Say 全可见）。

| 测试 | 层级 | 断言 |
| --- | --- | --- |
| `copy_mode::clean::tests::say_pair_headers_keep_preview_payload` | unit | 合并头 6 形态：closed/open、spinner 剥离、流式空 preview/no preview/空白 preview → `None`，有 preview → `Some(preview)` |
| `copy_mode::clean_say_tests::single_line_say_survives_copy_mode` | e2e（TestBackend） | 单行 Say（正文 Hidden）在 copy 视图完整可见；`Say(1 step)` label 不出现 |
| `copy_mode::clean_say_tests::multi_line_say_shows_first_line_and_rest` | e2e | 多行 Say 首行（头部 preview）与其余行（正文 Skip 后余量）都可见 |
| `copy_mode::clean_say_tests::streaming_say_preview_survives` | e2e | 流式中 preview 可选中复制，`running ` spinner 剥离 |
| `copy_mode::clean_say_tests::expanded_ladder_keeps_thinking_calls_and_full_say` | e2e | 展开阶梯的 step thinking、call 输出与完整 Say 在 copy 视图全可见 |

## Notes / Compatibility

- 纯净化层变更，历史会话 replay 与 Web/SPA 不受影响（SPA 无 copy 模式）。
- 净化行集 `CleanModel` 懒重建机制不变，滚动几何自动按新行集计算。

## Related Docs

- agents/tui/index.md（ChatBlock 合并对契约 + copy_mode LineKind 语义）
- features/index.md（Ctrl+G copy 模式条目）
- changelog/2026-09-04/tui-say-pair-merged-header.md（合并头与正文去重的引入方）
