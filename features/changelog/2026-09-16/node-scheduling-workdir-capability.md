Commit: 1ac64fe8b81a2c7c144c72b717a8031ab18f2589

# 多运行时宿主节点调度：隐藏工作空间并修正错误语义

multi-runtime host 自身不跑会话，每个 runtime 是独立进程、独立 data_dir 与启动 workdir，节点级 scheduling workdir 语义不成立（host 与 worker 两处一致拒绝）。此前 UI 对所有节点都展示并提交 workdir 字段，保存被 host 以 503 `host routing:` 前缀拒绝，误导为路由故障。本次按「设计正确、UI 越界」修复：

- host 与 worker 的 maintenance `scheduling` 读接口对 host_capacity 节点统一返回 `workdir:null,workdir_supported:false`；普通节点响应不变（隐含 `workdir_supported:true`）。
- host `configure_scheduling` 中 FIFO 与 workdir 两条能力校验由 `anyhow::ensure!`（被包装成 503 `host routing:`）改为直接 `RpcReply::error(400, ...)`，与 worker 侧 400 对齐。
- SPA `NodeSchedulingModal` 读取 `workdir_supported === false` 时隐藏 workdir 输入项并展示「多运行时宿主不支持节点级工作空间，会话目录由各运行时自身决定」，保存 payload 不携带 workdir；普通节点行为不变（保留 `workdir:null` 清空语义），读取失败回退快照时默认按支持 workdir 处理。
- `crates/web/spa/dist` 按惯例重建并随代码提交。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| host 读接口暴露能力标志、带 workdir 返回 400 且无 `host routing:` 前缀、LIFO 返回 400 | `host_scheduling_reports_workdir_unsupported_and_rejects_workdir_with_400` | `crates/agent/src/host/tests.rs` |
| host 节点不渲染 workdir 输入、提交体无 workdir 字段、展示不支持说明 | `multi-runtime hosts hide the workspace input and omit workdir from the payload` | `crates/web/spa/src/harness/management.dom.test.jsx` |

## 回归

- SPA 专项：`management.dom.test.jsx` 9 项通过（既有 3 个 workdir 用例保持通过）。
- SPA 全量：`npm test` → 104 files / 753 passed / 0 failed。
- SPA 构建：`npm run build` 通过；`scripts/check-spa-drift.sh` → no drift。
- 规则 02 分包回归：`cargo test -p opencoder-agent -p opencoder-worker -p opencoder-core -p opencoder-control --no-fail-fast` → 54 个测试二进制全 ok，800 passed / 0 failed（日志 `/tmp/reg2.log`）。
- 既有 host 用例回归：`three_runtime_versions_keep_live_model_calls_and_global_fifo`、`deployment_http_requires_authentication_current_host_and_current_server` 均通过。
