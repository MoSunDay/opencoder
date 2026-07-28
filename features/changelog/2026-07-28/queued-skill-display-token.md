Commit: (working-tree, pre-initial-commit)

# feat(tui): 纯技能提交在队列/steer 面板显示原始 `{$name}` token

## 背景

当用户提交一个纯技能 token（仅 `{$name}`、无附加文本）时，系统会生成一段冗长的
触发描述（`skill_trigger`：`The \`{name}\` skill is now active. Begin executing its
instructions immediately.`）作为 admit 到 store 的 prompt。这段触发文本是给 LLM 看的，
但此前也被原样推入 `steer_items` / `queue_items` 显示镜像——用户在侧边队列面板看到的
是冗长的英文触发句，而非自己实际输入的 `{$name}` token。

## 变更

### 显示层：新增 `skill_token_display` 并替换 Steer/Queue 两处显示镜像

- **`crates/tui/src/app_helpers.rs`**：新增纯函数 `skill_token_display(name) -> format!("{{${name}}}")`，
  返回用户原始提交的 token 字符串。`skill_trigger` 的完整触发文本仍照常 admit 到 store
  （LLM 需要它），仅显示镜像改用 token。
- **`crates/tui/src/app.rs`**：
  - Steer 纯技能路径：`steer_items.push((seq, trigger))` → `..skill_token_display(skill_name)`。
  - Queue 纯技能路径：`queue_items.push((seq, trigger))` → `..skill_token_display(skill_name)`。
  - re-export 列表增加 `skill_token_display`。
- **`crates/tui/src/app_tests.rs`**：新增 `skill_token_display_shows_dollar_token`，
  断言 `skill_token_display("repo-memory") == "{$repo-memory}"`。

### 不变项

- store 持久化路径不变（仍 admit 完整 `skill_trigger` 文本）。
- runner 消费逻辑不变。
- 普通文本提交的 `clean` 显示不变。

## 测试

隔离 git worktree（clean HEAD `a0bbe42` + 仅本任务 5 处改动）验证：

| 检查 | 结果 |
|------|------|
| `cargo build -p opencoder-tui --lib` | PASS — Finished |
| `cargo test -p opencoder-tui --lib` | PASS — **548 passed; 0 failed**（含新增 `skill_token_display_shows_dollar_token`） |
| `cargo clippy -p opencoder-tui --lib -- -D warnings` | PASS — 零警告 |

> 注：主工作树被多个并发 agent 的范围外改动污染（TUI lib test target 有 26 处编译错误，
> 涉及 AutoPilot config 字段 / SubagentSteer / API 签名变更等未完成特性），无法在主树
> 取得干净基线。本验证在独立 worktree 中完成，diff 经核对仅含上述 5 处预期改动，
> 无 `#[ignore]` / 删除测试 / 弱断言等修绿行为。
