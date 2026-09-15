Commit: c0b88d6131b493822219179b1135a0a01807822b

# 节点会话索引跨页恢复

节点会话超过 500 条时，索引续页原先只传 session ID，Store 要求 activity 游标，导致
`invalid list cursor` 并反复断开节点连接。现使用 `max(updated_at, created_at)|id`，
与 Store 排序和解码合同一致，支持时间戳并列及导入会话 updated_at 为零的情况。

修改仅涉及 worker 索引续页，不改变 DAG 或依赖分析业务。版本基于原在线
`ea2052b26a76e73c5814e8ce2a9acb8942de564a` 构建，保留此前发布能力，经标准平台发布脚本
部署四个二进制。原节点身份恢复 online/ready，2026-09-15 11:24:15 至 11:57 的观察窗口
未再出现游标错误，并完成 VTA 任务 `dep-3da2a9bf1bce380f32da57e2391546a8` 的实际 DAG 分析和回填。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 1001 会话、跨页并列时间戳、两次完整无重复回放 | `index_replays_all_session_pages_with_activity_order_and_ties` | `crates/worker/tests/internal_session_index/main.rs` |
| 运行中内部会话状态 | `internal_session_is_running_only_while_its_loop_is_live` | `crates/worker/tests/internal_session_index/main.rs` |
| 断连不终止已受理工作及恢复路由 | `server_routes_by_id_and_disconnect_does_not_stop_accepted_work` | `crates/worker/tests/fleet_channel.rs` |

专项测试 3 passed / 0 failed；平台 bundle 构建及部署 readback 通过。

- `cargo clippy --workspace --all-targets -- -D warnings`：退出码 0，零警告。
- `cargo build --workspace`：退出码 0。
- `cargo test --workspace --no-fail-fast -- --test-threads=1`：退出码 0，
  391 个测试目标、5187 passed / 0 failed / 6 ignored；6 项均为原有需挂载/NFS/runc 环境的手工用例，
  没有新增跳过。本次节点分页新增用例包含在通过分母中。

完整 gate 回执为 `/tmp/opencoder-index-recovery-gates.json`，对应原始日志为
`/tmp/opencoder-index-recovery-isolated-workspace-final.log`；两者及 Clippy/构建输出另存于
`/data00/viking-test-agent/var/acceptance/dependency-dag-closeout-20260915/node-regression/`（Git ignored）。

回归环境必须隔离宿主配置与数据目录：宿主配置指向只读归档目录，会使默认 Wasm pool 用例
误用线上路径。采用私有 mount namespace 的临时目录，不修改宿主或线上配置。
NFS 状态测试共享进程级导出注册表，其他用例持锁时非阻塞状态读取会按约定返回 stopped，
因此全量复核采用 `--test-threads=1`。测试入口还需使用 `/dev/null` 标准输入；
heredoc 启动器留下的空管道会被 `rg` 识别为输入源，导致目录检索用例返回空结果。
同一组 89 个 notepad 用例在空管道下有 3 项失败、在 `/dev/null` 下全部通过。
这些调整仅用于测试启动方式，没有修改业务代码、删减用例或降低断言。

## 相关文档

- [worker 模块](../../../agents/worker/index.md)
