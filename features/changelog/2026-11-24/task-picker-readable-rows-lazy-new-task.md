# `/task` 行回退标题、清洗动态文本且新任务不再预写 session 行

一句话：`/task` 的“+ New task”只切换内存会话，首条输入持久化前不写
session 行；列表第二行由 preview 回退到 title，动态文本经终端安全清洗并按
显示宽度判空，二者皆无可显示内容时显示固定占位，不再出现整列孤立的 `…`。

## 背景

- `TaskPick::New` 在选中时立即 `create_session` + `set_harness_runtime`，
  因此打开一个新任务、尚未输入任何内容（更未请求模型）就会在 `/task`
  留下无标题、无 preview 的空行。
- `/task` 每行第二行只渲染 `preview`。消息页不可读时列表走点查降级路径，
  该路径有意跳过 preview 子查询以避开损坏的 `messages` 页；于是所有行的
  preview 都是空串，界面统一退化成 `…`，而 `sessions.title` 明明可读。
- `extract_preview` 对超长/非文本首条消息也返回空串，同样触发该问题。
- 懒创建放大了既有状态丢失窗口：首条输入前经 `/requirement` 写入
  `update_session` 时行尚不存在（静默 no-op），随后懒创建又把
  `requirement` 写成 `None`。

## 变更

- `crates/tui/src/app_task.rs`：`TaskPick::New` 不再预写 session/runtime 行，
  返回普通 `SessionState`；首条输入由 session 层 `persist()` 懒创建并带上
  首条用户文本标题。
- `crates/session/src/lib.rs`：懒创建 session 行时持久化
  `self.requirement.clone()`，保住首条输入前通过 `/requirement` 设置的值。
- `crates/tui/src/task_row.rs`：新增可见任务文本选择——preview 优先，缺失时
  回退 title，二者皆空返回 `(no task text)`（14 列）。preview/title 均先经
  `terminal_text::sanitize_single_line` 清洗，再折叠连续空白为单行；清洗后
  显示宽度为 0（仅控制字符或零宽字符）的候选同样跳过。
- `crates/tui/src/task.rs`：渲染第二行时传入 `s.title` 作为回退；文本按固定
  56 显示列（`PREVIEW_W`：60 列弹窗减去边框、高亮符号与留白）截断，不随实际
  弹窗宽度动态变化。不改变两行布局与 `(current)`/age 头行。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 新任务在首条输入前不落 session/runtime 行 | `agent_self_detaches_remote_into_a_lazy_local_task_then_resumes_only_remote_context` | `crates/tui/src/app_task/switch.rs` |
| 首条输入前设置 requirement，懒创建行仍保留 | `lazily_created_session_row_preserves_requirement` | `crates/session/src/lib_tests.rs` |
| preview 与 title 同时存在时 preview 优先 | `preview_line_prefers_preview_when_both_preview_and_title_exist` | `crates/tui/src/task_row.rs` |
| 空 preview 回退 title / 空白 preview 同样回退 | `preview_line_empty_preview_falls_back_to_title` | `crates/tui/src/task_row.rs` |
| 无 preview 且无 title 时保持 ≥10 列占位 | `preview_line_without_preview_or_title_keeps_a_readable_placeholder` | `crates/tui/src/task_row.rs` |
| 仅控制字符/零宽字符的 title 回退占位 | `preview_line_skips_titles_with_no_visible_width` | `crates/tui/src/task_row.rs` |
| 多行任务文本折叠为单行 | `preview_line_collapses_multiline_text_to_one_line` | `crates/tui/src/task_row.rs` |
| 长 title 截断并带省略号 | `preview_line_truncates_long_title_to_width_with_ellipsis` | `crates/tui/src/task_row.rs` |
| 宽字符 title 截断不撕裂字形 | `preview_line_does_not_tear_wide_chars_in_title` | `crates/tui/src/task_row.rs` |
| 已有 preview 截断与宽字符截断回归 | `preview_line_truncates_to_width_with_ellipsis`、`preview_line_does_not_tear_wide_chars` | `crates/tui/src/task_row.rs` |
| 弹窗实际渲染 title 回退 / 空行占位 | `empty_preview_renders_title_fallback`、`empty_preview_and_title_render_a_readable_placeholder` | `crates/tui/src/task.rs` |

## 定向验证

- `cargo test -p opencoder-tui --lib task_row::tests` → 12 passed / 0 failed（1844 filtered out）。
- `cargo test -p opencoder-tui --lib task::tests` → 31 passed / 0 failed（1825 filtered out）。
- `cargo test -p opencoder-tui --lib agent_self_detaches_remote_into_a_lazy_local_task`
  → 1 passed / 0 failed（1855 filtered out）。
- `cargo test -p opencoder-tui --lib remote::subtask_parent::tests` → 2 passed / 0 failed（1854 filtered out）。
- `cargo test -p opencoder-session --lib lazily` → 1 passed / 0 failed（477 filtered out）。
- `cargo clippy -p opencoder-tui --all-targets --no-deps -- -D warnings` → 零警告。
- `cargo clippy -p opencoder-session --all-targets --no-deps -- -D warnings` → 零警告。
- `rustfmt --edition 2021 --check` 覆盖本次触达的 Rust 文件 → 无差异。
- 说明：以上 Clippy 均显式使用 `--no-deps`。不带该参数时 Clippy 会检查路径依赖，
  可能命中 `opencoder-store` 中与本次变更无关的既有未提交改动；本次以目标
  crate 的 `--no-deps` 结果为准。
- 测试范围：本次修改及其直接影响的 `/task` 新建、渲染与 session 懒创建链路；
  未运行全量测试。

## 已知限制（后续单独处理）

sidecar 在首条主任务输入前产生的 `LlmUsage` 事件，仍会因子会话行尚不存在而
被 `append_event` 的外键约束丢弃。优先级低，不在本次修复范围内。
