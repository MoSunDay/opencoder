Commit: 502c4db4 (working-tree)

# 平台发布自动恢复 Node admission

## 背景

Server 优雅停止时会持久化 Frozen admission，保护进行中的执行。发布后如果只重启进程，Node 可以用原有 node ID 重连，但会继续处于冻结状态，容易被误判为 token 失效或需要重新注册。

## 变更

- 新增 `scripts/platform/deploy.sh`，统一完成 token 一致性校验、bundle 原子安装、Server → Agent 滚动重启、原有 Node ID 重连校验和 admission reopen。
- Reopen 后再检查 `/api/ready`，确保节点真的恢复可调度。
- 保留旧二进制备份，失败时停在冻结状态，不删除执行数据或重新注册 Node。

## 验证

- Server 与 Agent 使用同一 token 文件内容；token 不写入命令行或日志。
- 重启后通过持久 Node ID 检查，Node 重新连接后由 `DELETE /api/admin/drain` 复开。
