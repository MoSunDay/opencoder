Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# server 模块

opencode-server 二进制：解析参数并启动 control 控制面。

## 关键路径
- `src/main.rs` — clap 参数（host/port/web/data-dir/token）→ `opencoder_control::serve`
- token 来源：--token/--token-file 或 `OPENCODER_SERVER_TOKEN`，缺省报错（不自动生成）
- 日志：默认 EnvFilter 含 `opencoder_control`；显式 `RUST_LOG` 优先

## 边界
- 不链接 session/team/project/DAG 执行引擎；执行全部在 opencode-agent。
- 只保存四字段运行索引并按 ID 查询所属节点。

## 相关
- [agents/control](../control/index.md) — 控制面实现
- [agents/web](../web/index.md) — HTTP/SSE 服务实现
- [agents/agent](../agent/index.md) — 节点执行二进制
- [Agent 平台](../../docs/agent-platform.md) — 部署
