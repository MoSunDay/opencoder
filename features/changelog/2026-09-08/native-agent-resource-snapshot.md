Commit: 本提交（基于 9639f553）

# 原生 Agent 执行保留接收时的资源快照

节点接收任务时已经复制了固定版本的 Agent 资源，但转入原生 HTTP prompt 入口后，配置重载会重新指向共享资源目录，随后 `tokio::spawn` 又丢失任务作用域。实际执行因此可能读取错误版本的提示词和技能，或因为只读 NFS 不支持执行权限而找不到工具。

`start_drain_locked` 现在捕获接收时的资源作用域，将其保留到恢复会话的配置和后台 drain 中。普通 Web prompt 继续使用自身配置。指定的快照缺失时明确报错，不回退到共享目录。该改动适用于原生 Agent 执行，没有加入业务分析引擎或工作流代理。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| HTTP prompt 使用固定快照中的提示词、技能正文和可执行工具 | `node_prompt_uses_pinned_prompt_skills_and_executable_tools` | `crates/web/tests/web_agent_snapshot.rs` |
| 普通 Web prompt 保持配置中的资源目录 | `unscoped_web_prompt_keeps_configured_agent_resources` | 同上 |
| 指定快照缺失时记录错误，且不调用 LLM | `missing_node_snapshot_fails_without_using_live_resources` | 同上 |

## 验证

- 定向测试：`cargo test -p opencoder-web --test web_agent_snapshot` → 3 passed / 0 failed。
- 全量回归：`cargo test --workspace --no-fail-fast -- --test-threads=8` → 4801 passed / 0 failed / 5 既有手工用例 ignored；333 条 `test result` 汇总。
- Lint：`cargo clippy --workspace --all-targets -- -D warnings` → exit 0，零警告。
- 构建：`cargo build --workspace` → exit 0。
- 行数：`handle.rs` 797 行，新测试文件 169 行；`git diff --check` 通过。

测试使用系统盘短路径 `TMPDIR=/var/tmp/oc-vdep`，在仅启用 loopback 的独立网络命名空间中执行，并移除测试进程的代理环境变量。本机 `/tmp` 位于低于节点健康门槛的数据盘，且主机临时端口范围与 83 个 Kubernetes NodePort 重叠，会使随机本地测试请求转发到其他服务；未修改生产容量策略、网络规则或测试断言。

本机原始输出保存在 `/var/lib/opencoder-integration/viking-dependency-analysis-20260908/` 下的 `workspace-tests-final.log`、`clippy-final.log`、`workspace-build.log`，汇总见 `test-counts.json` 和 `gate-results.json`。
