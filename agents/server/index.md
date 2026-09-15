Commit: 3c1222a5e61536ec96914a7edb40d246bc6665e6

# server 模块

opencoder-server 二进制：解析参数并启动 control 控制面。

## 关键路径
- `src/main.rs` — 普通模式启动控制面；`--release-config` 绑定版本与 Host/资源服务；`--resources` 仅启动独立只读资源服务。
- token 来源：--token/--token-file 或 `OPENCODER_SERVER_TOKEN`，缺省报错（不自动生成）
- 日志：默认 EnvFilter 含 `opencoder_control`；显式 `RUST_LOG` 优先

## 边界
- 不链接 session/team/project/DAG 执行引擎；执行全部在 opencoder-agent。
- 保存五字段运行索引，执行明细按 ID 回查所属节点；双版本共享控制库的持久请求回执。
- 发布退役仅等待本实例请求完成，不冻结节点；管理员 drain 是独立操作。版本 Server 监听本地端口，由固定 Nginx 入口切换。
- Unix 模式在监听前注册 USR2（发布）和 USR1（回滚），带本实例版本身份请求本机 Host 启动独立发布作业；处理失败不结束 Server。`signal_protocol: 1` 表示支持该入口。

## 相关
- [agents/control](../control/index.md) — 控制面实现
- [agents/web](../web/index.md) — HTTP/SSE 服务实现
- [agents/agent](../agent/index.md) — 节点执行二进制
- [Agent 平台](../../docs/agent-platform.md) — 部署
