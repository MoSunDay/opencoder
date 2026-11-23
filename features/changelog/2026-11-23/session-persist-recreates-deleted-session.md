# 会话行被删除后持久化自动重建

一句话：`SessionState::record` 在 `append_message` 命中外键约束时，把会话行视为
已被其他进程删除（或存储层失步），从内存状态 `INSERT OR IGNORE` 重建会话行并
重试追加一次，避免一条删除/失步把整回合持久化永久毒化。

## 背景

`/task clear all`、web clear、`dialogs_clear`、`session delete` 可能由另一个进程
删除当前运行中的 `sessions` 行。此后 `messages.session_id` 的立即外键约束会让
每次 append 失败；原实现直接 `?` 终止回合，且 `session_created` 仍为 true，
后续每次持久化都重复失败。

## 变更

- 抽出 `create_session_row`：复用原有建行逻辑（标题、agent/model、summary/
  handoff/skill、`set_harness_runtime`）；`create_session` 本身为
  `INSERT OR IGNORE`，不覆盖仍存在的行。
- `record` 的 append 仅在错误链为 FK 违例时：`session_created = false` → 重建
  会话行 → 再追加一次；其他错误原样上抛。
- 删除前已被 `ON DELETE CASCADE` 清掉的旧消息、inputs、events、subagent_tasks、
  TODO workflow 等子行不会随会话行恢复；内存 transcript 不被截断。
- 空闲会话删除后保持删除；运行中会话在下一次持久化时复活。删除 API 签名不变。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 跨进程 clear 后重建并继续追加，旧消息不回来 | `record_recreates_a_session_cleared_by_another_process` | `crates/session/tests/persist_recreates_deleted_session.rs` |
| 显式 delete 后下一次 record 重建 | `record_recreates_an_explicitly_deleted_session` | 同上 |
| FK 错误识别（依赖） | `foreign_key_violation_is_detected_through_anyhow_context` | `crates/store/src/store/errors.rs` |

- 测试范围：用户已在提交前完成定向测试；本次按指示免测提交，未重复运行。
- 未运行全量测试。
