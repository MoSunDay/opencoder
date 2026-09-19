Commit: 5a08ace988ce8f5973ac5a81034b891187be1058

# 信号发布 rel-5a08ace9（Agent 提交修复生效）

## Context

线上此前运行 `rel-0662b924`。本次发布包含 Agent 创建受理超时窗口、首条
Agent prompt 随会话创建提交、SSE 首屏回放，以及调度/评审流程的已验证改动。

## Change Summary

- 通过 `--signal` 从 `rel-0662b92479d7d82df82a604491c9ca15876bf2ad` 切换到
  `rel-5a08ace988ce8f5973ac5a81034b891187be1058`；回执 phase=complete，当前
  journal phase=complete，candidate=null。
- 发布包 `/srv/releases/rel-5a08ace9` 的四个 launcher、SPA 摘要和 SHA256SUMS
  校验通过；健康接口返回 `0.1.0 (5a08ace9)`。
- 发布前完成在线 SQLite 备份：`/var/backups/opencoder/pre-5a08ace9-20260918`。

## Impact Surface

- Control、Runtime、Host 和 launcher 已使用 5a08ace9；在途旧 Runtime、真实模型
  shell、资源服务进程在切换前后保持同一 PID。
- 4 个在线节点均 ready，14 个 DAG 定义保持可读；调度模式为 open，inflight
  admissions=0。

## Validation

- 回归门：Clippy、workspace build、workspace test 均通过；Rust 5,368 passed /
  0 failed / 7 ignored，SPA 114 files / 843 passed。
- 真实平滑发布验收 PASS：15 分钟、171 个发布后 DAG 样本、89 个切换期间请求；
  最大接收间隔 0.197s，最大调度间隔 0.21s，SSE 重连 0.102s，无流量失败。
- Agent canary 的首条 prompt 均持久化并得到预期回复；一次节点通道短断时，执行
  按 durable assignment 继续并完成，同一 ID 重试返回 200 且未重复执行。证据目录：
  `/var/tmp/opencoder-release-20260918/5a08ace9`。

## Notes / Compatibility

同一 execution ID 的受理回执和幂等重试保持有效；节点暂时断开时应查询 receipt
和 session，而不是用新 ID 重复提交。未执行数据库数据删除或鉴权凭据变更。

## Related Docs

- [Agent 提交受理超时](agent-submit-create-timeout.md)
- [Agent 首条 prompt 随创建提交](agent-first-prompt-rides-session-create.md)
- [Agent 调度平台](../../agent-platform/index.md)
- [control 模块](../../../agents/control/index.md)
