Commit: (working-tree, pre-initial-commit)

# feat(tui): /task 选择器展示 session 激活的 skill 标签

## 背景

`/task` 选择器列出全部 sessions 时每行只显示 id/agent/状态，看不出各 session
当前激活的 skill。用户需要在切换会话前就知道"这个 session 在跑什么 skill"。
store 的 `list_sessions` 此前不返回 `sessions.skill` 列，TUI 无法渲染。

## 变更

### store 面：list_sessions 携带 skill body（`crates/store/src/libsql_store/sessions.rs`、`types.rs`）

- **`sessions.rs`**：`list_sessions` 的 SELECT 追加 `s.skill` 列（`subagent_cancelled`
  子查询后），行映射 `skill: r.get::<Option<String>>(9)?`。
- **`types.rs`**：`SessionListItem` 加 `pub skill: Option<String>` —— 存 skill **body**
  （完整指令文本），供 TUI 取名展示。

### TUI 面：TaskPicker 渲染 skill 标签（`crates/tui/src/task.rs`）

- 新增 `TaskPicker::skill_tag` 渲染逻辑：session 有 skill body 时，先用
  `discover_skills()` 匹配出技能名（`{$name}`），匹配不上则回退取 body 首行；
  无 skill 时标签留空。badge/后缀文本共存时标签位稳定（`survives_badges_and_suffix_tags`）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 标签：匹配技能名紧邻模式 chip | `skill_tag_renders_matching_name_next_to_mode_chip` | crates/tui/src/task.rs |
| 标签：未发现时回退 body 首行 | `skill_tag_falls_back_to_first_body_line_when_not_discovered` | crates/tui/src/task.rs |
| 标签：无 skill 不渲染 | `no_skill_tag_when_session_has_none` | crates/tui/src/task.rs |
| 标签：badge/后缀共存稳定 | `skill_tag_survives_badges_and_suffix_tags` | crates/tui/src/task.rs |
| store：list_sessions 携带 skill body | `list_sessions_carries_skill_body_for_picker_tag` | crates/store/tests/store_integration.rs |

- 全量回归：`cargo test --workspace` → 102 binaries，**1543 passed / 0 failed / 1 ignored**（当次实跑）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：`task.rs` 662 ≤ 800（迭代）；`sessions.rs` 225、`types.rs` 272（迭代 ≤800）

## Impact Surface

- **可感知影响**：`/task` 选择器每行显示激活 skill 的标签（技能名或 body 首行）；
  `list_sessions` 返回值新增字段，CLI `session list` 等消费方不受影响（新增字段向后兼容）。
- **不影响**：skill 激活/注入机制、drain 语义、web/CLI 调用面。

## Related Docs

- [agents/store](../../../agents/store/index.md)
- [agents/tui](../../../agents/tui/index.md)
- [既有相关 changelog](../2026-07-31/fix-tui-task-switch-cancel-before-quit.md)
