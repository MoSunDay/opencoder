Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# opencode-agents — 版本化 Agent 写路径与 NFS 只读导出

## 关键路径

- <agents_root>/prompts|skills|tools|memory/<名>/v{n} — 共享资源池，版本只增、多 agent 共用
- <agents_root>/<agent>/meta.json — 引用卡：current 四字段资源名 + history
- <agents_root>/active — 生效 agent 单行 marker
- crates/core/src/agent/meta.rs — 卡片/active 读写、validate_agent_name
- crates/core/src/agent/mod.rs — resolve_agent；effective_default 四级链
- crates/core/src/agent/compose.rs — compose_prompt（Soul/How/Output）
- crates/core/src/skill.rs — discover 多根遮蔽，first-wins
- crates/agents/src/write.rs — save_resource_version/create_agent_with_profile
- crates/agents/src/rollback.rs — rollback_resource：切 current 指针
- crates/agents/src/references.rs — scan_resource/references_snapshot
- crates/agents/src/io.rs — atomic_write（temp+fsync+rename）
- crates/agents/src/nfs.rs — 只读 NFSv3 导出，变更全 NFS3ERR_ROFS
- crates/agents/src/serve.rs — spawn_nfs_server + default_opts_from_config
- crates/web/src/api_agents.rs — /api/agents CRUD 与激活
- crates/web/src/api_agent_resources.rs — 资源版本上传/回滚
- crates/web/src/api_agent_nfs.rs — /api/agents/nfs 生命周期
- crates/web/spa/src/agentsConfig.jsx — Agent 配置页

## 边界

- 读路径在 core agent::{meta,resource,compose}，本 crate 仅写路径 + NFS
- crates/agent 是节点 worker 二进制，另一个 crate

## 相关

- [agents/core](../core/index.md)
- [agents/session](../session/index.md)
- [agents/web](../web/index.md)
- [Agent Harness](../../features/harness/index.md)
