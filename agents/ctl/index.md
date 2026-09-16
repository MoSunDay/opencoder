Commit: f2d723ed2a32a5a394eac05f58bc5558e7cfe08f

# ctl 模块

`opencoder-cli`：Server API 远程管理客户端。

## 索引
- `src/lib.rs` — clap 命令分发
- `src/http.rs` — `RequestPlan` + Bearer + 退出码（0/1/2/4/64）
- `src/cmd/*.rs` — 按域子命令（纯 plan() 映射）
- `tests/` — 子命令→RequestPlan 契约与集成 e2e
