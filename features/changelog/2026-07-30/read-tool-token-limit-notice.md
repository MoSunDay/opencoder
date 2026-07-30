# feat(session/tools): read 工具在 token 上限截断时输出可继续的 notice

## 背景

`read` 工具按预估 token 预算（`MAX_TOKENS = 5000`）边读边截断：当 token 超限时
提前停止，实际终点 `actual_end` 小于请求范围 `requested_end`。此前该截断是
「静默」的——仅靠 metadata footer 的 `lines_read`（小于请求 `limit`）间接反映，
模型需主动比对才能察觉未读完整；既有测试甚至显式断言输出「不包含」任何截断提示
（`assert!(!out.contains("[INCOMPLETE READ]"))`）。

缺少一个明确、可操作的信号，告知模型「内容被 token 上限截断，请从 offset N 继续」。

## 变更

### 行为（`crates/session/src/tools/read.rs`）

1. 新增截断判定 `token_capped = actual_end < requested_end`：仅当 token 预算导致
   提前停止（而非正常行数分页或文件读完）时为真。
2. `token_capped` 为真时，在 metadata footer 前追加一行，给出精确的可继续 offset：
   ```
   [INCOMPLETE READ] output truncated at token limit; re-read with offset=<actual_end+1> to continue.
   ```
3. 正常分页停止（命中 `limit`、文件已读完）**不**触发 notice——属预期行为，不应
   被误报为「未完整读取」。
4. `description()` 同步说明：notice 仅在 token 上限截断请求范围时出现。

改动隔离于 read 工具输出层：不触及 metadata footer 的字段/形状、行号格式、tab 展开、
Store / ChatStream / runner 契约（read.rs 不构造 `SessionMeta`、不依赖 store）。

### 回归测试（`crates/session/src/tools/read.rs::tests`，11 → 12）

- `test_token_limit`：断言由 `!contains` 翻转为 `contains`，并新增
  `assert!(out.contains("offset="))`——token 上限截断现在触发 notice。
  （随行为变更的正确断言翻转，非弱断言修绿。）
- 新增 `test_offset_remaining_content_no_notice`：offset=51、limit=100 读取仍有
  400 行剩余的文件，命中行数上限停止——断言**不**出现 notice，覆盖「正常分页不误报」。
- `test_offset_pagination`、`test_metadata_no_more` 维持 `!contains`：行数分页与
  文件读完均不触发 notice。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| token 上限截断触发 notice + 可继续 offset | `test_token_limit` | `crates/session/src/tools/read.rs` |
| 行数上限分页（仍有剩余）不触发 notice | `test_offset_remaining_content_no_notice` | `crates/session/src/tools/read.rs` |
| 行数分页（offset 翻页）不触发 notice | `test_offset_pagination` | `crates/session/src/tools/read.rs` |
| 文件读完不触发 notice | `test_metadata_no_more` | `crates/session/src/tools/read.rs` |

### 回归

| 检查 | 结果 |
|------|------|
| `cargo build -p opencoder-session` | PASS — Finished（read.rs 生产代码编译干净）|
| `cargo clippy -p opencoder-session --lib -- -D warnings` | PASS — 零警告 |
| `cargo test -p opencoder-session --lib tools::read` | PASS — **12 passed; 0 failed; 0 ignored**（read 模块 11→12，本会话实跑）|
| `cargo test --workspace` | PASS — 全部测试套件通过（本会话 act 模式实跑）|
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS — 零警告（本会话 act 模式实跑）|
| 防修绿扫描 | PASS — 无 `#[ignore]`、无删测试、无弱断言修绿、无调试输出 |

> 注：规划阶段（read-only plan 模式）记录的全量 workspace 回归当时**编译失败**——工作树中
> 另一并发任务正落地 store/session 的 `task_type` 重构（`SessionMeta` 新增 `task_type` 字段后，
> 多个集成测试 target 的构造点存在重复字段 / 字段错位，报 `E0062`/`E0063`）。该改动与本 read
> 变更**无因果**：read.rs 不构造 `SessionMeta`、不依赖该字段。**本会话（act 模式）实测：该并发
> 重构已补齐转绿——`cargo test --workspace` 全部测试套件通过、`cargo clippy --workspace
> --all-targets -- -D warnings` 零警告（见上表两行），read 变更的全量回归闭环完成。**

## Impact Surface

- read 工具在 token 上限截断时多一行 `[INCOMPLETE READ]` notice（含可继续 offset）。
- 不影响：metadata footer 字段与形状、行号/tab 展开格式、行数分页/offset 语义、
  Store / ChatStream / runner / web / cli / tui。
- 被 explore/build subagent 调用 read 的契约：footer 不变，仅多一条可选 notice，
  不破坏既有解析。

## 行数

- `crates/session/src/tools/read.rs`：300 行（新增文件 ≤ 400；迭代中 ≤ 800）✓
