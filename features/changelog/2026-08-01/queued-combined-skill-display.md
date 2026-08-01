Commit: (working-tree, pre-initial-commit)

# fix(tui): 排队提交「{$skill} + 其他需求」时 skill 插入信息不再从队列面板消失（含 resume 闭环）

## 背景

用户报告：**queued 路径**下如果输入同时含 `{$skill}` token 和其他需求（如
`{$repo-memory} 修复 main.rs 的 bug`），"skill 插入信息会消失"。

排查结论：效果链路（drain 时 system prompt 注入 skill body）本身是健全的——
`KeyAction::Queue` / 运行中 `KeyAction::Submit`（同样以 `Delivery::Queue` 入队）
经 `resolve_persist` 在入队时激活 `skill_handle`（与 worker 共享的
`SessionState::skill_prompt` Arc）并落盘 `sessions.skill`，drain 侧由
`run_one_llm_call` 每 turn 重读该 Arc（`crates/session/tests/skill_mid_run.rs` 已证明）。
**消失的是展示信息**：store 只入队 token 剥离后的 clean 文字（LLM / web drain
不得见 token），而排队项唯一的用户可见面是侧边队列面板 + `queued:` 消费标记，
二者显示的都是剥离后的 clean 文字——`{$skill}` 从 UI 上完全消失。对比 Submit
路径（transcript 记录原始输入、token 可见），排队路径行为不一致，正是
"skill 插入信息会消失"。

## 变更

### 队列/插队项展示保留原始输入（`crates/tui/src/skill_display.rs`、`app.rs`）

- **`skill_display.rs`**：新增纯函数 `queued_item_display(text, clean)`——token
  剥离改变了文本时返回原始输入（trim），否则原样返回 clean（纯文字提交零变化）。
  Submit 路径的 transcript 本就记录原始输入，此函数让排队面板/消费标记与之对齐。
- **`app.rs`**：三处组合提交（运行中 Submit 入队、Steer 插队、Tab-Queue）的
  `queue_items` / `steer_items` 展示串由 `clean` 改为 `queued_item_display(&text, clean)`；
  纯 skill 提交仍走 `skill_token_display`（`{$name}`），行为不变。store 入队内容
  保持 clean，LLM 永不看到 token。经 `pub(crate) use` 重导出（app.rs:796）。

### resume / 重载展示闭环（`session_inputs.display_text` 列，v6 迁移）

- **`schema.rs`**：`SCHEMA_VERSION` 5→6，`session_inputs` 加可空 `display_text TEXT`
  列（`migrate()` `if from < 6` 块，`add_column_if_absent` 幂等；旧行保持 NULL）。
- **`types.rs`**：`SessionInput` 加 `display_text: Option<String>`（serde default，
  bundle 新旧格式双向兼容，`FORMAT_VERSION` 无需 bump）。
- **`inputs.rs`**：INSERT/SELECT/`row_to_input`/`row_to_input_full` 映射新列（索引
  0-7 不变，display_text 取 index 8）。`claim_next_queue` 仍只把 `prompt` 交给
  runner —— LLM 永远读 clean，display_text 只是展示串。
- **`app_helpers.rs`**：`mk_input_with_images` 增加 `display_text` 参数；app.rs 三处
  admit 时组合提交传 `Some(queued_item_display(&text, &clean))`、纯 skill 传
  `Some(skill_token_display(skill_name))`，prompt 仍为 clean。
- **`queue_panel.rs`**：新增 `pending_mirror`（display_text 回退 prompt）+ 
  `restore_pending_mirrors`（按 `pending_inputs` 恢复面板镜像）；`app_task.rs` 的
  `/task` 切换重载与 `app.rs` run_app 启动（quit→resume）共用，重启后排队/插队项
  恢复显示原始输入（含 token），seq 与 drain retain 机制匹配。
- **测试文件拆分**（行数 gate）：`store_integration.rs` 1555→780（≤800），
  并发/wal/事务测试迁至新文件 `store_concurrency.rs`（391）、schema 迁移测试迁至
  `store_migrations.rs`（371），subagent 测试并入 `subagent_status_counts.rs`；
  纯搬运零语义改动（对齐 commit 921c02d 先例）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 展示：纯文字原样透传 | `plain_text_passes_through_clean` | crates/tui/src/skill_display.rs |
| 展示：`{$skill}+text` 保留 token | `combined_skill_keeps_token_visible` | crates/tui/src/skill_display.rs |
| 展示：仅空白差异用 clean | `whitespace_only_difference_uses_clean` | crates/tui/src/skill_display.rs |
| 展示：文中多处 token 保留 | `mid_text_token_preserved` | crates/tui/src/skill_display.rs |
| 效果：排队组合提交 drain 时 skill 入 system prompt | `queued_combined_submission_drains_with_skill` | crates/tui/tests/queued_skill_drain.rs |
| 持久化：display_text roundtrip | `display_text_roundtrip` | crates/store/tests/display_text.rs |
| 持久化：旧行 NULL 回退 prompt | `display_text_none_falls_back` | crates/store/tests/display_text.rs |
| 迁移：v5→v6 加列幂等 | `v5_to_v6_migration_adds_display_text` | crates/store/tests/display_text.rs |
| 契约：claim 只交 clean prompt、display_text 保留 | `claim_next_queue_keeps_prompt_clean_with_display_text` | crates/store/tests/display_text.rs |
| 持久化：bundle 导入导出保留 display_text | `bundle_roundtrip_preserves_display_text` | crates/store/tests/display_text.rs |
| 重载：镜像用 display_text 回退 prompt | `pending_mirror_uses_display_text_with_prompt_fallback` | crates/tui/src/app_helpers_tests/mod.rs |
| 重载：启动恢复双面板 | `restore_pending_mirrors_restores_display_text_at_reload` | crates/tui/src/app_helpers_tests/mod.rs |
| 透传：mk_input_with_images 带 display_text | `mk_input_with_images_passes_display_text` | crates/tui/src/app_helpers_tests/mod.rs |
| e2e：resume 恢复原文 + drain 保持 clean | `resume_restores_display_originals_and_drain_stays_clean` | crates/tui/tests/resume_queue_display.rs |

- 全量回归：`cargo test --workspace` → 102 binaries，**1543 passed / 0 failed / 1 ignored**（当次实跑；ignored 为既有 `research_smoke_bing_wikipedia`，需真实 Chrome/网络）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误
- 行数：`display_text.rs` 304、`store_concurrency.rs` 391、`store_migrations.rs` 371、`resume_queue_display.rs` 140（新文件 ≤400）；`store_integration.rs` 780、`app.rs` 800、`app_helpers.rs` 785、`queue_panel.rs` 372（迭代 ≤800）

## Impact Surface

- **可感知影响**：Tab-排队 / 运行中 Enter 入队 / Steer 插队的 `{$skill} <任务>`
  组合提交，队列面板与 `queued:`/`steer:` 消费标记显示原始输入（含 token）；
  quit→resume 或 `/task` 切换后，pending 排队/插队项在面板恢复显示原文，不再丢失。
- **不影响**：LLM/web 契约（drain 与 `claim_next_queue` 只读 clean `prompt`，
  display_text 不进模型；store 入队内容仍 clean）、skill 生效机制（system prompt
  注入，`skill_mid_run` 契约）、bundle 格式（serde default 双向兼容）。
- 边界说明：旧库存量行 display_text 为 NULL，重载时回退显示 clean prompt
  （既有行为，升级后新提交的项才有原文展示）。

## Related Docs

- [agents/tui](../../../agents/tui/index.md)
- [既有相关 changelog](../2026-07-30/queued-combined-skill-persist.md)
