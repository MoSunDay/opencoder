# Resume 上下文持久化：plan→act 移交、技能、subagent 状态跨重启保留

## 背景

`opencode -s`（headless resume）与 `/task`（TUI 中 plan→act 移交）切换时丢失四类上下文，违反「会话可中断、可恢复」的核心承诺：

1. **Gap A — plan→act 移交不落库**：`plan_handoff::handoff()` 只折叠内存 transcript，不记录边界。重启后 `resume()` 把完整 plan-mode 历史当 act 输入回放，上下文窗口被 planning 噪音淹没、焦点丢失。
2. **Gap B — 技能不持久**：`SetSkill` 只改内存 `skill_prompt`，不写 session 行。重启后技能静默丢失。
3. **Gap C — subagent 事件批量尾部 flush**：`run_subagent`/`replay_child` 用 buffer+tail-flush，子代理被中断/出错时全部进度丢失（fire-and-forget detached spawn，仅靠 `sleep(200ms)` 的侥幸）。
4. **Gap D — headless resume 无 subagent 状态摘要**：`run_headless` 重连后不汇报挂起/完成的子代理，用户对 session 进度零可见。

## 变更

### Schema v3 迁移（`crates/store/src/libsql_store/schema.rs`）

`SCHEMA_VERSION` 2→3。`CREATE_SESSIONS` 新增三列 `handoff_seq INTEGER`、`handoff_plan TEXT`、`skill TEXT`（均 nullable）。`migrate` 增加 v3 分支：对既存库 `ALTER TABLE sessions ADD COLUMN ...`。迁移是纯加列、nullable、向后兼容。

### 类型层（`crates/store/src/types.rs`）

`SessionMeta` 与 `SessionPatch` 各加 `handoff_seq: Option<i64>`、`handoff_plan: Option<String>`、`skill: Option<String>` 三字段。全仓库 `SessionMeta` 字面量（web/api.rs、cli/run.rs 含 fork+3 测试、cli/session_cmd.rs、tui/web/session/store 测试）同步补齐。

### Store CRUD（`crates/store/src/libsql_store/sessions.rs`）

`INSERT` / `SELECT` / update handler / `row_to_meta` 四处扩展三列读写。

### Gap A — 移交边界记录与 resume 重建

- `crates/session/src/lib.rs`：`SessionState` 加 `handoff_seq`/`handoff_plan` 字段 + `after_handoff(seq, plan)` 方法。
- `crates/session/src/plan_handoff.rs`：`handoff()` 调 `after_handoff()` 记录边界；抽出 `handoff_message(plan_display) -> Message`（合成 `Role::User`+`synthetic=true` 的执行指令）。
- `crates/session/src/resume.rs`：`resume()` 若 `meta.handoff_seq` 有值，则 trim `[0..handoff_seq]` 并 `insert(0, handoff_message(plan))` 重建焦点 transcript；用 `else if` 使**移交主导于压缩**（dominant transcript reset，二者互斥）。恢复 `handoff_seq`/`handoff_plan` 到 `SessionState`。

### Gap B — 技能持久与恢复

- `resume()` 从 `meta.skill` 恢复 `skill_prompt`。
- `crates/tui/src/worker.rs`：`SetSkill` 与 `SwitchAndStart` 经 `SessionPatch` 落库 `skill` / `handoff_*`。
- `crates/tui/src/app.rs`：`KeyAction::SetSkill` best-effort 持久化技能。
- `crates/tui/src/session_ui.rs`：`replay_into_chat` 从 `meta.handoff_plan` 推入 `ChatBlock::Plan`，resume 时展示移交 plan 卡片。

### Gap C — 增量 subagent 事件持久

`crates/session/src/runner.rs::run_subagent`（~L766-828）与 `crates/session/src/resume.rs::replay_child`：旧 buffer+tail-flush 换成 `tokio::sync::mpsc::channel` + 单 flusher task，串行 `append_event` 保序消费。`on_event` 回调是 sync `FnMut`，故用 channel 解耦 async `append_event`。`flusher.await` 在返回前完成，保证子事件在父 `run(...)` 返回瞬间即持久（**无需 sleep**）。旧的 `subagent_persists_child_events_to_store` 测试中的冗余 `sleep(200ms)` 已删除（flusher 设计下无需，同一路径已被更强断言的 `subagent_child_events_persisted_before_return` 覆盖）；中断/出错时保留已 flush 的部分进度。

### Gap D — Headless resume 子代理摘要

`crates/cli/src/run.rs`：`run_headless` 在 resume 后调 `print_resume_summary(&session).await`，输出蓝字单行 `⤷ resumed session: done/total subagents done — ✔explore … ✘build …`。格式逻辑抽出为纯函数 `pub(crate) fn format_resume_summary(&[SubagentTaskRecord]) -> Option<String>`（空→`None`），便于单测；`print_resume_summary` 仅 fetch+`eprintln!("\x1b[34m{line}\x1b[0m")`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| Gap A — 移交后 resume 重建焦点 transcript（plan 指令 + act 消息，丢 planning 噪音） | `resume_after_handoff_reconstructs_focused_transcript` | `crates/session/tests/handoff_resume.rs` |
| Gap A/B — `handoff_seq`/`handoff_plan`/`skill` 三列经 create/update/get 往返 | `session_handoff_and_skill_fields_round_trip` | `crates/store/tests/store_integration.rs` |
| Gap B — resume 恢复持久化技能 | `resume_restores_persisted_skill` | `crates/session/tests/skill_resume.rs` |
| Gap B — 无技能 resume → None | `resume_without_skill_has_none` | `crates/session/tests/skill_resume.rs` |
| Gap C — 子事件在 `run` 返回瞬间即持久（无 sleep）、seq 严格升序唯一 | `subagent_child_events_persisted_before_return` | `crates/session/tests/subagent.rs` |
| Gap D — resume 子代理摘要格式（done/total 计数、✔/✘/… 字形、prompt 截断、空→None） | `format_resume_summary_lists_subagents` | `crates/cli/src/run.rs`（lib 单测） |
| Schema v2→v3 迁移（`migrate(conn,2)` 仅跑 `if from<3` 分支，加 handoff/skill 三列，旧行 nullable） | `schema_migration_v2_to_v3_adds_handoff_and_skill` | `crates/store/tests/store_integration.rs` |

- 全量回归：`cargo test --workspace` → **679 passed / 0 failed**（基线 672，+7 新增）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` → 零错误

## 行数 Gate

新文件均 ≤400 行：`handoff_resume.rs`=139、`skill_resume.rs`=105。修改的大文件（`runner.rs`=1043、`app.rs`=1043、`store_integration.rs`=1173）为既存大文件，本次仅做外科手术式小改（runner.rs 的 mpsc flusher 约 60 行、app.rs 的 SetSkill 持久化 1-2 行、store_integration.rs +1 测试），未引入新的失控增长。

## 安全 Gate

无硬编码密钥 / 凭证 / token / 连接串。
