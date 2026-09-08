Commit: (working-tree, 基于 65c9d891ae905e7925277d29a87cd8e7957e8dad)

# server — opencoder-server 二进制

`crates/server` 解析监听地址、工作目录和 token，然后调用 [control](../control/index.md) 启动平台 Web 和调度。凭据来源为参数、`OPENCODER_SERVER_TOKEN` 或启动时生成。

默认日志同时包含 `opencoder_control`，节点协议拒绝等控制面错误可直接从 Server 日志排查；显式 `RUST_LOG` 仍按调用者配置。Server/Node 协议需成套升级或回滚。

它不构建 Web 本地 session AppState，也不链接 session/team/project/DAG 执行引擎。全部执行分配给 [agent](../agent/index.md)，Server 只保存四字段运行索引并按 ID 查询所属节点。部署见 [Agent 平台](../../docs/agent-platform.md)。
