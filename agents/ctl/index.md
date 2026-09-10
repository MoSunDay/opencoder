Commit: b465f440381bd009dc9bd3a8192ad88eab44cede

# ctl 模块

opencoder-cli：opencode-server 控制面的远程管理客户端。

## 关键路径
- `src/lib.rs` — clap `Cli` 全局 flag + 命令分发：health/exec/session/nodes/dag/todo/project/brain/teams/agents/raw
- `src/ctx.rs` — `Ctx` 解析：flag > `OPENCODER_SERVER_URL`/`OPENCODER_SERVER_TOKEN` > 报错
- `src/http.rs` — 纯数据 `RequestPlan` + Bearer `send` + `exit_code`（401/403→2，其余非 2xx→4）
- `src/out.rs` — stdout 恰一个 JSON 文档；人读信息与结构化失败只进 stderr
- `src/sse.rs` — 自含 SSE 客户端 `FrameAssembler`，流式帧打紧凑单行 JSON
- `src/cmd/mod.rs` — 共享执行器 `exec_plan`（缓冲）/`exec_stream`（SSE，非 2xx 先预检）
- `src/cmd/*.rs` — 按域模块：clap Subcommand + 纯 `plan()` + 薄 `run()`
- `src/cmd/raw.rs` — `raw` 逃生舱：任意 method+path 直发，限 7 个方法，`--json` 支持 `@file`
- `src/main.rs` — tokio 包裹 `run()`；无子命令退出码 64
- `tests/parse_*.rs` — 子命令→RequestPlan 纯映射契约
- `tests/server_local.rs`、`tests/server_node.rs` — 进程内真 control / 带脚本化 WS 节点的集成 e2e

## 边界
- 纯客户端：运行时仅依赖 opencoder-core（其余通用库），不链接 session/store/worker。
- 退出码：0 成功 / 1 传输失败 / 2 认证失败 / 4 服务端拒绝 / 64 无子命令。
- `--build-info` 在读取 Server 地址与凭据前返回构建元数据。

## 相关
- [agents/control](../control/index.md) — 所调用的平台 API
- [agents/server](../server/index.md) — 服务端二进制
