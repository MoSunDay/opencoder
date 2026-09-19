Commit: 896013049fe3bd0f3384c52e9638e3a7107aa6fc

# ctl 模块

`opencoder-cli`：Server API 远程管理客户端。

## 索引
- `src/lib.rs` — clap 命令分发
- `src/http.rs` — `RequestPlan` + Bearer + 退出码（0/1/2/4/64）
- `src/cmd/*.rs` — 按域子命令（纯 plan() 映射）；`dag dispatch --input <JSON>`
  merge 进 body 的 `input` 键（同键优先 `--input`），驱动 worker 侧
  「执行要求」prompt 注入与 `<node>/dag/<run>/input.json`
- `tests/` — 子命令→RequestPlan 契约与集成 e2e

## Brain 契约

`brain plan-defs`、`brain runs`、`brain library` 与 Web 共用 API。`brain runs` 可创建 v3、读取最小快照、轮次 operation 索引和调度事件，并发送 pause/resume/cancel；`brain activate-local` 同时严格解析 v2 图上下文和 v3 调度上下文。v2 输入、旧决策树写入及旧派发路径按既有迁移/只读规则处理。

参见 [brain](../brain/index.md) 与[运行协议](../../docs/brain-orchestration.md)。
