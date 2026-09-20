Commit: 896013049fe3bd0f3384c52e9638e3a7107aa6fc

# ctl 模块

`opencoder-cli`：Server API 远程管理客户端。

## 索引
- `src/lib.rs` — clap 命令分发
- `src/http.rs` — `RequestPlan` + Bearer + 退出码
- `src/cmd/` — 按域子命令（纯 plan() 映射）
- `src/cmd/brain.rs` — brain 子命令，与 Web 共用 API
- `tests/` — 子命令→RequestPlan 契约与集成 e2e

## 相关
- [brain](../brain/index.md)、[运行协议](../../docs/brain-orchestration.md)
