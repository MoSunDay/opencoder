Commit: 7bc123c7

# 节点准入重连与菜单收起展开

## Context

节点在优雅停止时会持久化 `Frozen` 准入状态。若 Server 已重新开放，节点重连仍可能携带旧状态，导致节点列表显示 `node admission is frozen`，不能再次调度。

## Change Summary

- `crates/control/src/transport/socket.rs` 在节点 WebSocket 接入时按 Server 当前状态同步 `Freeze` 或 `Reopen`，并校验同步回执。
- `DELETE /api/admin/drain` 即使 Server 已是 `open` 也会向在线节点发送 `Reopen`，修复已在线节点遗留的冻结状态。
- SPA 侧边菜单增加“收起菜单 / 展开菜单”按钮，收起后保留图标菜单，展开后恢复分类切换和文字菜单。

## Validation

- `cargo test -p opencoder-control --test admission`：4 passed。
- `cargo test -p opencoder-control --test e2e`：171 passed。
- `npm test -- --run src/app.dom.test.jsx`：21 passed。

## Related Docs

- [Agent 调度平台](../../agent-platform/index.md)
- [control 模块](../../../agents/control/index.md)
- [web 模块](../../../agents/web/index.md)
