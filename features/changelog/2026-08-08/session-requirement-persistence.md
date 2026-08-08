Commit: (working-tree, pre-initial-commit)

# session requirement 持久化 + /requirement 斜杠命令

## 背景
会话需要一份可编辑的「任务需求」文本（任务描述/验收标准），随会话持久化并在
resume 后存活。此前该文本仅存在于运行时内存，压缩 / 重启后丢失。本次引入
`requirement TEXT` 列（schema v8 迁移）与 `/requirement`（别名 `/req`）斜杠命令，
在 plan 编辑器内编辑并落库。

## 变更
### store —— 持久化与 schema 迁移
- **`crates/store/src/libsql_store/schema.rs:4`**：`SCHEMA_VERSION` 7 → 8。
- **`crates/store/src/libsql_store/schema.rs`**（CREATE_SESSIONS）：`sessions` 表新增 `requirement TEXT`（新建库直接具备）。
- **`crates/store/src/libsql_store/schema.rs:249`**：`migrate` 新增 `if from < 8` 分支，`add_column_if_absent(conn, "sessions", "requirement", "TEXT")` —— 已存在的 v7 库首次打开即补列，避免 INSERT/SELECT 缺列报错。
- **`crates/store/src/types.rs:48,89`**：`SessionMeta.requirement` / `SessionPatch.requirement`（均 `Option<String>`）。
- **`crates/store/src/libsql_store/sessions.rs`**：INSERT_SESSION / SELECT_SESSION 携带 requirement。

### tui —— /requirement 命令与编辑器
- **`crates/tui/src/command.rs:37,185,204`**：`/requirement`（别名 `/req`）→ `SlashAction::Requirement`。
- **`crates/tui/src/plan_edit.rs:26,53,127`**：`EditKind { Plan, Requirement }`；`PlanEdit::new_requirement`、`enter_requirement`；requirement 编辑器标题 `edit requirement`、绿色边框。
- **`crates/tui/src/chat_req.rs`**（新增 78 行，从 chat.rs 拆出）：`ChatView::last_requirement_text()`（优先 `requirement_text`，回退首条用户 prompt）、`update_requirement_text()`（`\r\n` → `\n` 消毒）。
- **`crates/tui/src/chat_types.rs`**：`ChatView` 新增 `requirement_text` / `first_prompt`；`ChatBlock` 加 `#[allow(clippy::large_enum_variant)]`（新字段使 variant 超阈值，递归 UI 模型不宜盲目 box）。
- **`crates/tui/src/app_loop.rs:633,685,709`**：dispatch `enter_requirement`；plan/requirement-edit 模式按键处理；Exit 时仅当改动才落库。

### session / web —— 透传字段
- **`crates/session/src/resume.rs:207`**：`SessionState` 构造携带 `requirement.clone()`。
- **`crates/web/src/api.rs:36,260`**：两处 `SessionMeta` 构造补 `requirement`。

### 测试适配
- 全仓 ~137 处 `SessionMeta { ... }` 构造补 `requirement: None`（或 `..` 展开继承）；修复脚本嵌套导致的 `}SessionMeta {` 拼接损坏（16 文件）。
- `crates/tui/src/plan_edit.rs`：修正被 `EditKind` 插入打断的孤立 doc comment（clippy）。

## 测试覆盖
| 功能 | 测试名 | 文件 |
|------|--------|------|
| v7→v8 迁移补 requirement 列 | `schema_migration_v7_to_v8_adds_requirement` | `crates/store/tests/store_migrations.rs:464` |
| requirement 文本优先级/回退 | `last_requirement_text` 等 5 例 | `crates/tui/src/chat_req.rs` |
| /requirement 解析与分发 | `parse_requirement_full`/`alias`/`dispatch_requirement` | `crates/tui/src/command.rs` |
| SessionMeta 全库构造 | 全量回归 | (57 文件) |

- 全量回归：`cargo test --workspace --no-fail-fast` → 2084 passed; 0 failed; 0 ignored
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 0 warnings
- build：`cargo build --workspace` → 干净
- 行数：chat_req.rs 78 ≤ 400；plan_edit.rs 295、chat_types.rs 172 ≤ 800

## Impact Surface
- 新增 `/requirement`（`/req`）命令；`sessions.requirement` 列。
- 已有 v7 库首次打开自动迁移到 v8（补列），对用户透明。
- 不影响：CLI 子命令、Web SSE 协议、store trait 接缝、subagent 调度。

## Related Docs
- [agents/store](../../agents/store/index.md) — schema 迁移（含 v7→v8）
- [agents/tui](../../agents/tui/index.md)
