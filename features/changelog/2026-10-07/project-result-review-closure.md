Commit: 07d29e11815cdbbbfc82a73a5a208bd0eb313e62

# 历史执行结果的版本化读取

稳定 Host 从原所属 Runtime 的持久化目录提供版本化分块，独立 Worker 使用同一读取器。读取历史执行不唤醒、恢复或迁移 Runtime，不改写原生结果。

Team 长文件使用有界快照缓存，首次流式计算摘要，后续按偏移读取；文件替换或原位修改使快照失效。读取器校验节点、执行类型与路径身份，拒绝符号链接。

旧 Runtime 曾只保存 Agent 输出的 8 KiB 尾部，新读取器保留实际落盘内容，不声称恢复已丢弃文本。项目仅保存执行引用，读取通过原生详情和分块 API 完成。

## 测试覆盖

| 功能 | 测试入口 |
| --- | --- |
| 旧 Runtime、新 Host、执行中升级、休眠、重启与回滚；HTTP 分块及引用不缓存结果 | [result_upgrade.rs](../../../crates/agent/src/host/tests/result_upgrade.rs) |
| 64 MiB Team 只计算一次摘要，文件替换和原位修改失效、身份与符号链接拒绝 | [result_reader/tests.rs](../../../crates/worker/src/result_reader/tests.rs) |

本次合并的完整验证见 2026-10-09 仓库清理记录；跨版本用例需显式提供旧发布二进制。

相关：[Host](../../../agents/agent/index.md)、[Worker](../../../agents/worker/index.md)。
