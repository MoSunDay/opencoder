Commit: 187ee827bad0cb2ae0b1900284b1a20176706166

# ctl 模块

`opencoder-cli`：Server API 远程管理客户端。

## 索引
- `src/lib.rs` — clap 命令分发
- `src/http.rs` — `RequestPlan` + Bearer + 退出码（0/1/2/4/64）
- `src/cmd/*.rs` — 按域子命令（纯 plan() 映射）
- `tests/` — 子命令→RequestPlan 契约与集成 e2e

## Brain 契约

`brain plan-defs`、`brain runs`、`brain library` 与 Web 共用 v2 API，输入 JSON 可携带具名 Markdown 文档。隐藏的 `brain activate-local` 是节点有限激活边界，按 v2 上下文调用同一内核；旧决策树写入、预览及派发命令报迁移错误，历史读取保留。

参见 [brain](../brain/index.md) 与[运行协议](../../docs/brain-orchestration.md)。
