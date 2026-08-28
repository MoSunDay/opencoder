Commit: (working-tree, store_integration 按职责拆分 + 具名测试取证规范落规)

# store_integration 按职责拆分 + 具名测试取证规范落规

## 背景

`crates/store/tests/store_integration.rs` 增长到 871 行，超迭代 800 行上限（先于 sandbox 轮存在，上轮评审问五遗留卫生项）。另将上轮评审问三的取证假信号教训（按测试名过滤独立测试文件静默零命中）沉淀为测试规范；`seed_builtin_skills` 不 clobber 的升级操作提示补入 release 记录。

## 实现

- **store**：`tests/store_integration.rs` → `tests/store_integration/` 目录目标——`main.rs`（模块声明）+ `common.rs`（`conv`/`fresh`/`make_session` 共享 helper）+ 按职责 6 模块（`sessions`/`messages`/`transactions`/`listing`/`bundle`/`events`）。17 个测试函数逐一保留、断言零改动，cargo 测试目标名不变（`cargo test -p opencoder-store --test store_integration` 照常可用），最大文件 `sessions.rs` 214 行（≤400）。
- **rules/03**：新增「具名测试取证规范」小节——独立测试文件必须按目标运行（`--test <file>`），按函数名过滤会全目标 `0 passed; filtered out` 静默零命中；目录目标用例名带模块前缀（`模块名::函数名`）；无 `test result:` 行即为无效取证。
- **release 记录**：`sandbox-mode-replace-plan-act.md` 边界节补「升级提示」——旧机器存量 `~/.opencoder/skills/task-plan`、`~/.opencoder/skills/review` 不会自动获得 question 解锁契约（seed 不 clobber），升级后删除这两目录重启一次即自动 re-seed。
- **记忆修复**：`agents/store/index.md` 代表性验证条目同步目录目标与新职责边界（WAL 并发压力在 `store_concurrency.rs`）。

## 遗留观察（无代码动作）

- e2e/长套件时长方差：`act_clear_context_fold` 历史 0.07s–52s 波动（均绿），CI 侧关注稳定性即可。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 会话 CRUD 全生命周期契约 | `sessions::create_get_update_delete_session_contract` | `store/tests/store_integration/sessions.rs` |
| 消息 roles/blocks/usage 往返 | `messages::append_and_load_preserves_all_roles_and_blocks` | `store/tests/store_integration/messages.rs` |
| 部分失败事务回滚 | `transactions::transaction_rollback_on_partial_failure` | `store/tests/store_integration/transactions.rs` |
| 取消事务不 panic 且 store 可用 | `transactions::cancelled_transaction_does_not_panic` | `store/tests/store_integration/transactions.rs` |
| 游标分页 + 搜索过滤 | `listing::list_pagination_with_metadata` | `store/tests/store_integration/listing.rs` |
| bundle 导入导出往返 | `bundle::bundle_export_import_roundtrip` | `store/tests/store_integration/bundle.rs` |
| 事件追加 + after 重放 | `events::events_append_and_after_replay` | `store/tests/store_integration/events.rs` |

（17 个用例全数保留，此处列代表性映射；拆分前后 `--test store_integration` 均 `17 passed; 0 failed`。）

- 全量回归：`cargo test --workspace` → 233 suites / 3308 passed / 0 failed（与上轮基线精确闭合）
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- 构建：`cargo build --workspace` → 零错误
- 行数：新增 .rs 文件最大 214（≤400）；超 800 文件清零（原 871 行文件已拆分）
