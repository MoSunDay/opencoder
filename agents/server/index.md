Commit: (working-tree, 基于 c1a1b2e78e1ccd4a3cc2ac6dc408a76d30bf46e6)

# server — opencoder-server 二进制

`crates/server` 解析监听地址、工作目录和 token，然后调用 [control](../control/index.md) 启动平台 Web 和调度。凭据来源为参数、`OPENCODER_SERVER_TOKEN` 或启动时生成。

它不构建 Web 本地 session AppState，也不链接 session/team/project/DAG 执行引擎。全部执行分配给 [agent](../agent/index.md)，Server 只保存四字段运行索引并按 ID 查询所属节点。部署见 [Agent 平台](../../docs/agent-platform.md)。
