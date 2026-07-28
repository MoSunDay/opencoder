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

- **`crates/tui/src/skill_display.rs`**（新模块）：抽离 `skill_trigger` 与
  `skill_token_display` 两个纯函数到此独立模块。`skill_token_display(name)` 返回
  `format!("{{${name}}}")`——用户原始提交的 token 字符串。`skill_trigger` 的完整触发
  文本仍照常 admit 到 store（LLM 需要它），仅显示镜像改用 token。
  - 抽离目的：`app_helpers.rs` 在引入 `skill_token_display` 后达到 809 行（>800 迭代上限），
    将两个同族 skill 显示/触发 helper 移出后回到 790 行。
- **`crates/tui/src/app_helpers.rs`**：移除 `skill_trigger` / `skill_token_display`
  （已迁移至 `skill_display.rs`）。
- **`crates/tui/src/lib.rs`**：新增 `pub mod skill_display;`。
- **`crates/tui/src/app.rs`**：
  - Steer 纯技能路径：`steer_items.push((seq, trigger))` → `..skill_token_display(skill_name)`。
  - Queue 纯技能路径：`queue_items.push((seq, trigger))` → `..skill_token_display(skill_name)`。
  - import 改为从 `crate::skill_display` 引入 `skill_token_display, skill_trigger`。
- **`crates/tui/src/app_tests.rs`**：新增 `skill_token_display_shows_dollar_token`，
  断言 `skill_token_display("repo-memory") == "{$repo-memory}"`；既有
  `skill_trigger` 测试引用改为 `crate::skill_display::skill_trigger`。
- **`crates/tui/src/app_helpers_tests/mod.rs`**：`skill_trigger` 引用改为
  `crate::skill_display::skill_trigger`。

### 不变项

- store 持久化路径不变（仍 admit 完整 `skill_trigger` 文本）。
- runner 消费逻辑不变。
- 普通文本提交的 `clean` 显示不变。

## 测试

| 检查 | 范围 | 结果 |
|------|------|------|
| `cargo build --workspace` | 当次主树 | PASS — Finished |
| `cargo test --workspace` | 当次主树 | PASS — **1267 passed; 0 failed**（含并发 agent 范围外测试） |
| `cargo test -p opencoder-tui --lib` | 当次主树 | PASS — **565 passed; 0 failed** |
| `cargo clippy -p opencoder-tui --all-targets` | 当次主树 | PASS — 本任务文件零警告（`key_handler_plan_edit_tests.rs` dead-code 来自并发 agent 未跟踪文件，非本 diff） |
| 防修绿 diff 扫描 | 本任务 | PASS — 无 `#[ignore]`、无删除测试、无弱断言 |

> 注：本任务净增 +1 测试（`skill_token_display_shows_dollar_token`）。主工作树当前被
> 多个并发 agent 改动（AutoPilot / SubagentSteer / API 签名等未完成特性），上述主树数字
> 包含其范围外测试；本任务自身回归隔离验证基线 = 547 → 548（+1），数学成立。
