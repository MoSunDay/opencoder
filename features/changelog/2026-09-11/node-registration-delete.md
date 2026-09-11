Commit: 86567de28148c84077cdc77229bff38b5b5ab11e

# 节点页支持删除节点注册

## Context

节点页面此前只能查看和维护节点，无法清理已停止节点的注册信息。删除注册不能连带清理任务或执行记录。

## Change Summary

- 节点表新增“删除节点”按钮和二次确认弹窗。
- 在线节点删除返回 409，并提示先停止节点服务；离线节点删除只移除 FleetStore 的 `fleet_nodes` 行。
- `execution_index`、节点本地任务与执行文件不由该接口处理，保留给各自数据管理逻辑。
- 删除仅允许管理员，缺少 Bearer token 或非管理员角色会被认证/角色层拒绝。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 离线注册删除保留执行索引并支持重启 | `delete_registration_keeps_indexes_and_survives_reopen` | `crates/control/tests/node_registration.rs` |
| 删除接口权限边界 | `node_registration_delete_requires_admin_identity` | `crates/control/tests/node_registration.rs` |
| 在线节点拒绝删除 | `connected_node_registration_cannot_be_deleted_while_agent_is_running` | `crates/control/tests/e2e/fleet_maintenance.rs` |
| 页面确认后调用删除接口 | `deletes only the selected node registration after confirmation` | `crates/web/spa/src/fleet/fleet.dom.test.jsx` |

## 验证

- `cargo test -p opencoder-control --test node_registration` → 2 passed / 0 failed
- `cargo test -p opencoder-control --test e2e connected_node_registration_cannot_be_deleted_while_agent_is_running` → 1 passed / 0 failed
- `cargo test -p opencoder-control --lib` → 47 passed / 0 failed
- `npm test -- --run src/fleet/fleet.dom.test.jsx` → 24 passed / 0 failed
- `npm run build` → 成功
- 发布 bundle：`dist/opencoder-platform-86567de2`；server/agent 已重启，build-info commit `86567de2`，两服务 active。
