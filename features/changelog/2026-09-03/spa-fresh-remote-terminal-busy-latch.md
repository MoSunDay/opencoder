# SPA fresh-remote 终帧 busy 挂死修复（上轮评审 TODO-2）

Commit: 8fcb9d9

## 背景

上轮 web 审查（F1–F4 收敛）的正向核对发现一处同根残留：节点上**首个**远程派发（`dialogSel` 为空）的终帧（done 与 error 同病）不复位 busy——终帧 effect 早退守卫含 `!dialogSel`，而 `sendRemote` 仅在已有选中时才回填 `dialogSel`。Sender 挂 loading，直到用户点开任一对话触发 `openDialog → resetTranscript`。缓解存在，体验缺口真实。

## 变更

`crates/web/spa/src/chat.jsx`，两侧同修：

- **终帧 effect 解耦 busy 释放与选中态**：`done`/`error` 一律 `setBusy(false)`+`setConnecting(false)`（busy 生命周期只属于 `send()`，任何终帧都必须放行 composer）；仅 `reloadAfterDone` 保留 `dialogSel` 守卫（无选中则无 store 重载对象）。
- **`sendRemote` 无条件回填 `dialogSel`**：派发产生的新 session（`sessionId !== dialogSel`，含首派发的 `null` 情形）一律前置进会话列表并选中——与 `sendLocal` 新建路径对称。回填经核实无副作用：`Conversations` 受控 `activeKey` 变更不触发 `onActiveChange`（仅用户点击/快捷键路径），不会误启 `openDialog` 重置刚开的流。

配套修一个测试基建盲区：`chat.dom.test.jsx` 的 fetch mock 中，宽泛的 `/api/nodes` 分支排在 `/events`、`/tasks` 之前，导致远程任务事件流（`/api/nodes/tasks/:id/events`）与派发 POST（`/api/nodes/:id/tasks`）都被吞成 `{nodes:[]}`——远程流此前在测试里根本无法驱动。路由顺序调整为 `/events` → `/tasks` POST → `/api/nodes` 兜底。

## 测试清单

| 场景 | 用例 | 位置 |
|---|---|---|
| 首个远程派发终帧放行 composer + 回填选中触发 store 重载 | `releases the composer when a FIRST remote dispatch reaches a terminal frame (no dialog selected yet)`（负验：仅回退 sendRemote 回填即红，busy 释放由防御层保持绿） | crates/web/spa/src/chat.dom.test.jsx |
| SPA 全量回归 | vitest 110/110（13 文件） | crates/web/spa |
| Rust 全量回归 | `cargo test --workspace`（含 tui 收敛后首次全量门） | workspace |
