# Fork 身份修复、隔离契约校正与 wss 形态覆盖

一句话：修复 fork 会话继承父执行 Linux 账户导致其首轮工具全部 `权限不够` 的缺陷（fork 环境本应 provision 全新账户）；把 O7 隔离测试中过期的快照 `0600` 断言校正为现行 `0440` 契约（per-execution identity 自 3e7292a2d 起 enforced 默认）；补齐 `serve_release` 第二层 metrics 凭据守卫的直接单测；新增 `node_tls` 套件以真实 rustls 终结器覆盖节点 `https→wss` 信道形态（注册、hello、报表、心跳全链路 + 明文拒绝）。

## 变更清单

- `crates/worker/src/operations/fork.rs`：fork 复制的 harness runtime 会带着父会话的 `run_as`，而 initialization 只在字段为空时回填，导致 fork 轮次仍降权到父账户（无法进入 fork 自己 0700 的 home/workspace）。现在 fork 落库时以新环境 provision 的账户覆写 `harness.run_as`（identity 关闭时同步写回 `None`），与“fork 永不沿用父账户”的既有注释对齐。
- `tests/operator_e2e/isolation.rs`：冻结快照模式断言 `0600`→`0440`（节点属主、只读组可读、任何人不可写），并同步模块头注释；该契约自 per-execution identity enforced 默认化后即为线上行为，旧断言属过期残留。
- `crates/control/src/bootstrap.rs`：新增 `serve_release_rejects_metrics_token_equal_to_the_admin_token` 单测，覆盖 `bootstrap.rs` 处 CLI 前置检查之后、此前从 CLI 不可达的第二层拒绝分支。
- `crates/control/tests/node_tls/`（新目录）：`main.rs` 单文件套件 —— rcgen 生成 CA + 127.0.0.1 服务证书，`SSL_CERT_FILE` 走生产 native-roots 信任路径，rustls acceptor + hyper http1（带 upgrade）终结 TLS 驱动 Server 路由，证明 `https://` remote 的 wss 信道注册/报表/心跳在线且明文被拒。
- `crates/control/Cargo.toml`：测试专用 dev-deps（rcgen/rustls/rustls-pemfile/tokio-rustls/hyper/hyper-util，ring-only 特性，不引入 aws-lc-rs）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| fork 使用全新 Linux 账户（home/cwd 探针落盘、父工作区无泄漏、重启后保持） | `remote_forks_preserve_conversation_type_and_policy_after_restart` | `tests/operator_e2e/fork.rs` |
| 冻结快照节点属主 0440 契约 | `operator_execution_isolates_home_and_workspace` | `tests/operator_e2e/isolation.rs` |
| `serve_release` 拒绝 metrics 凭据等于管理员 token（第二层守卫） | `serve_release_rejects_metrics_token_equal_to_the_admin_token` | `crates/control/src/bootstrap.rs` |
| wss 信道全链路（注册 + hello + 报表 + 在线心跳） | `wss_channel_registers_reports_and_stays_online_over_tls` | `crates/control/tests/node_tls/main.rs` |
