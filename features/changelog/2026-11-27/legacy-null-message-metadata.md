# Resume 诊断与会话切换错误上下文：legacy 可空消息元数据兼容 + 严格列精确定位

一句话：`/task` Resume 失败原先只回显裸错误（现场为 `Conversion failure: Null Value` 一类），没有
表 / 行 / 列定位。本轮分三层处理：(1) `messages` 的元数据列 `usage_json` / `synthetic` 按 NULL 兼容
读取（默认 usage / `false`），作为独立的 legacy 兼容修复；(2) 其余严格列不默认、不跳过，失败时带
session / message / column 定位；(3) 补齐 `/task` 切换链路的诊断与读取窗口——`sessions` 行解码补
session / column 上下文、Resume 错误带 session id、镜像 Store 恢复原生 OFFSET 尾部读取（压缩头部的
坏行不再拖垮只读尾部的 Resume）。

**边界（重要）**：现场的 Resume 失败经只读排查证实为存储引擎层的数据页损坏与畸形扫描，**不是**已证实
的可空列为 NULL；可空解码**不能修复**实际的数据损坏，损坏库需要走备份恢复 / 重建（仓库外的数据运维）。

## 背景与决策

- 触发条件（通用）：`/task` Resume / Fork 读取目标会话失败时，UI 只回显裸错误，不带表 / 行 / 列定位，
  无法区分「表形不含 NOT NULL 约束、行内写入了 NULL」「行级取值异常」与「物理页损坏」三类原因。
- **实际排查结论（只陈述已证实的事实）**：对受影响库做的全部是**只读**检查——完整性检查 / 页级扫描，
  以及按与 Resume 相同的 Turso（Cargo.lock 锁定版本 0.8.3）读取路径逐列探查的只读探针。这些检查证实
  的是 **B-tree / 页级损坏与畸形扫描**：逐列探查观察到的是引擎报损坏或直接 panic，**并没有**复现出
  一个解码层可达的 `Conversion failure: Null Value`。因此：
  - 现场失败的直接原因是物理数据损坏，**不是**已证实的「legacy 可空列为 NULL」；受影响行各列的真实
    取值（包括内容列是否为 NULL）**没有证据，本记录不作断言**。
  - 实际数据损坏**无法靠可空解码修复**：NULL 兼容只在引擎成功返回一行之后才生效，对损坏的页 / 畸形
    的扫描无能为力；修复损坏数据需要备份恢复或重建，不在本轮范围。
  - 「`usage_json` / `synthetic` 按 NULL 兼容读取」是一个**独立且正当的兼容性修复**：覆盖「表形不含
    NOT NULL 约束、行内写入了 NULL」这一类通用场景，与上述损坏无关，也不能被当作对现场问题的修复。
- 本轮不引入任何默认值填充或整行跳过来掩盖损坏：内容列与 `sessions` 的 `id` / `created_at` /
  `updated_at` 保持严格——伪造时间戳会打乱 `/task` 排序并移动 Resume 边界，跳过整行则会让会话凭空
  消失；损坏行继续硬失败，但错误精确到 session / message / column，便于定位。请求 id 由调用方传入，
  因为行自身的 `id` 列可能正是不可读的那一列。
- `ts_mirror` 的 `load_messages_after` 必须显式委托：`Store` trait 默认实现是「全量 load + 内存
  drain」，会把压缩头部（soft-deleted）的行也反序列化一遍。Resume 的压缩路径本来就用 OFFSET
  只读尾部、按设计不加载头部，所以头部坏行不该让只读尾部的 Resume 失败。这是**恢复既定读取窗口**，
  不是对损坏内容放行：同一会话的全量读取仍保持严格（见测试）。
- 决策：错误只记录 session id 与错误链，不记录标题 / 提示词 / 消息内容；用户可见行为不变（失败
  仍走原 mode flash 提示），仅新增可定位的诊断。

## 行为与实现

- `crates/store/src/turso_store/conversation/messages/parts/part_2.rs`：
  `row_to_message(session_id, row)` 先解码 `id`，再交给 `decode_message_columns` 解码其余列。
  `usage_json` 以 `Option<String>` 读取（NULL 或非法 JSON 均回退 `MessageUsage::default()`，非法
  JSON 保持 warn）；`synthetic` 以 `Option<i64>` 读取（NULL 按 `false`）。每列使用 `with_context`，
  错误形如 `decode messages.<column> for session <id> message <id>`。
- `crates/store/src/turso_store/conversation/messages/parts/part_1.rs`：
  `load` / `load_after` 调用 `row_to_message` 时传入 `session_id`。
- `crates/store/src/turso_store/conversation/sessions/parts/part_2.rs`：
  `row_to_meta(requested_id, row)` 对 `id` / `created_at` / `updated_at` 保持严格并带上下文，错误形如
  `decode sessions.created_at for session <id>`（与 messages 侧格式一致）。
- `crates/store/src/turso_store/conversation/sessions/parts/part_1.rs`：`get` 把请求 id 传入
  `row_to_meta`。
- `crates/tui/src/app_task.rs`：`load_session_for_switch` 用 `task switch resume for session <id>`
  包装 Resume 错误；新增 `switch_error`，在切换失败处记录 warn（仅 session id + 错误链）并返回
  用户可见的 mode flash 文本。
- `crates/tui/src/app.rs`：切换失败分支改为调用 `app_task::switch_error`（保留 `pending.pick` 以
  读取 session id），行数不变。
- `crates/tui/src/ts_mirror.rs`：覆盖 `load_messages_after` 直接委托 inner，保留原生 OFFSET 路径。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 可空 `usage_json` / `synthetic` 经 `load_messages` 解码为默认值且不丢消息 | `legacy_nullable_usage_and_synthetic_decode_with_defaults` | `crates/store/tests/schema_bootstrap/suite_3.rs` |
| 同一兼容行为覆盖 `load_messages_after` 的 OFFSET 尾部读取路径 | `legacy_nullable_usage_and_synthetic_decode_with_defaults` | `crates/store/tests/schema_bootstrap/suite_3.rs` |
| 正常行（含 usage、synthetic=true、display）行为不变 | `legacy_nullable_usage_and_synthetic_decode_with_defaults` | `crates/store/tests/schema_bootstrap/suite_3.rs` |
| 内容列（`blocks_json`）为 NULL 时保持严格报错，并包含 session / message / column | `strict_content_columns_report_session_message_and_column` | `crates/store/tests/schema_bootstrap/suite_3.rs` |
| `sessions` 的 NULL `created_at` / `updated_at` 保持严格失败，错误含 session 与列名；健康行与缺失 id 行为不变 | `null_session_stamps_report_session_and_column` | `crates/store/tests/schema_bootstrap/suite_4.rs` |
| 镜像 Store 保留 OFFSET 尾部路径：被跳过的压缩头部含 NULL 内容列时 `load_messages_after` 成功且只返回健康尾部；同会话全量读取仍严格报错 | `load_messages_after_keeps_the_offset_tail_path` | `crates/tui/src/ts_mirror_tests.rs` |
| `load_session_for_switch` 失败错误含 `/task` 切换上下文与 session id，且底层原因仍在错误链中 | `load_session_for_switch_error_names_the_session` | `crates/tui/src/app_task/tests.rs` |
| `switch_error` 返回完整错误链文本（有 / 无 session id 两种 pick） | `switch_error_keeps_the_cause_chain` | `crates/tui/src/app_task/tests.rs` |

## 实际执行

- 格式：`cargo fmt --check -p opencoder-store` → 无差异；`cargo fmt --check -p opencoder-tui` → 无差异。
- Clippy：`cargo clippy -p opencoder-store --all-targets -- -D warnings` → 零警告；
  `cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告。
- 定向回归：`cargo test -p opencoder-store --test schema_bootstrap` → 11 passed / 0 failed（含 `suite_3`
  2 例、`suite_4` 1 例）。
- 定向回归：`cargo test -p opencoder-store --test load_messages_after` → 2 passed / 0 failed。
- 定向回归：`cargo test -p opencoder-tui --lib ts_mirror::tests::` → 9 passed / 0 failed（含新增
  `load_messages_after_keeps_the_offset_tail_path`）。
- 定向回归：`cargo test -p opencoder-tui --lib app_task::tests::switch_error_keeps_the_cause_chain`
  → 1 passed / 0 failed。
- 定向回归：`cargo test -p opencoder-tui --lib app_task::tests::load_session_for_switch_error_names_the_session`
  → 1 passed / 0 failed。
- 定向回归：`cargo test -p opencoder-tui --test session_switch_restore` → 3 passed / 0 failed。
- 反向验证：临时移除 `ts_mirror::load_messages_after` 委托后，
  `load_messages_after_keeps_the_offset_tail_path` 按预期失败
  （`decode messages.blocks_json for session 01LEGACY message head-corrupt`，01LEGACY 为测试夹具 id），
  恢复委托后重跑 `ts_mirror::tests::` → 9 passed / 0 failed。
- 测试范围：本次修改及其直接影响链路；未运行全量测试。

## Impact Surface

- **修复边界**：本轮改动不修复、也无法修复存储层已损坏的数据页 / 畸形扫描；NULL 兼容解码仅对
  「引擎成功返回、且 `usage_json` / `synthetic` 为 NULL」的行生效。损坏库的恢复属于仓库外的数据运维。
- Store：`messages` 行解码对 `usage_json` / `synthetic` 的 NULL 兼容（默认 usage / `false`）；
  其余列与 `sessions` 的 `id` / `created_at` / `updated_at` 保持严格，失败带
  session（/ message）/ column 上下文。无默认值填充、无整行跳过、无数据改写。
- TUI `/task`：Resume/Fork 失败的错误链含 session id 与 `task switch resume for session <id>`；
  切换失败处新增一条 warn 日志（仅 session id + 错误链，不含标题 / 提示词 / 消息内容）；
  用户可见提示与失败处理行为不变。
- TUI 会话读取：ts 镜像 Store 的 `load_messages_after` 恢复原生 OFFSET 尾部读取，压缩头部不再被
  反序列化；全量读取保持严格。
- 未改公共 API / CLI / 依赖 / Ontology 目录；未触碰运行数据库。排查所用的临时只读诊断样例与临时
  输出不属于交付物，收尾时已删除、未入库。
