Commit: 896013049fe3bd0f3384c52e9638e3a7107aa6fc

# ctl 模块

`opencoder-cli`：Server API 远程管理客户端。

## 索引
- `src/lib.rs` — clap 命令分发
- `src/http.rs` — `RequestPlan` + Bearer + 退出码
- `src/cmd/` — 按域子命令（纯 plan() 映射）
- `src/cmd/brain.rs` — brain 子命令，与 Web 共用 API
- `src/cmd/brain/ontology.rs` — `brain runs create` 只放行显式 `schema_version` 3|4（其余在计划期报错，绝不回落 v2）；`brain runs layered <id>` / `brain runs layered-round <id> <round>` 读 v4 画布视图与层明细；隔离激活（`activate-local`，容器内调用）按 `schema_version` 走 v3 调度或 v4 分层画布，v4 空 `nodes` 走收口指令
- `tests/` — 子命令→RequestPlan 契约与集成 e2e

## 相关
- [brain](../brain/index.md)、[运行协议](../../docs/brain-orchestration.md)
