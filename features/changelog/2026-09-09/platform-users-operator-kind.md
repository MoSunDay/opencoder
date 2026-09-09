Commit: (working-tree, 基于 64f7000f5...)

# 平台用户体系与 Operator 执行类型

## 行为

- 控制面引入平台用户：`admin`/`root`/`user` 三角色，凭 Bearer token 认证。seed token 恒等 `admin`（不依赖用户表，constant-time 比较）；用户 token 以 sha256 哈希落库，明文 `oc_`+双 ULID 仅创建时返回一次。无 token 仍返回 401。
- `GET /api/me` 返回当前身份 `{name, role}`；SPA 登录收敛为 token-only：输入 token 后探测 `/api/me` 落地身份，导航按角色过滤（非 admin 落地 topics，仅 admin 可见「后台管理」与用户管理抽屉、Operator 子 tab）。
- Admin 用户管理：`GET /api/users` 列表（不含哈希/明文）、`POST /api/users`（重名 409、非法角色/名称 400）、`DELETE /api/users/:name`（自删 400、最后一个 admin 400、删除即吊销 token）。
- 角色权限矩阵（role_gate 纯函数）：admin 全量；user/root 仅 `GET /api/me`、`GET /api/nodes`、executions 读、`POST /api/executions` 与命令（仅 operator 类型，handler 级二次校验 403）。
- 新增 `ExecutionKind::Operator`：worker 以宿主机进程直接运行 agent 循环（无 runc 沙箱、无 node_maintenance 工具），默认标题「Operator」，首条用户消息注入宿主机 Operator 前导约束；inspect/命令/事件/详情等会话链路全量接入。
- 旧节点未注册 operator 时放置失败收敛为常规 503（`no ready online node can accept this execution`），PROTOCOL_VERSION 不升级。

## 接口与兼容

- 新增端点：`GET /api/me`、`GET|POST /api/users`、`DELETE /api/users/:name`。
- Schema v23 → v24：新表 `platform_users(name PK, token_hash UNIQUE, role, created_at)`；`Store` trait 新增用户查找/写入/删除方法（带默认空实现，仅 LibsqlStore 落地）。
- `auth_mw` 支持以 `Arc<dyn Store>` 做 token 哈希反查；`exempt` 路径（健康检查等）保持公开。认证关闭时视为 admin（本地部署行为不变）。
- e2e Harness（`build_app(state, Some(TOKEN), true)`）与 `new_state` 不建 admin 用户行，seed token 走常量比较路径。

## 功能 → 测试名

| 功能 | 测试 |
| --- | --- |
| Identity/Role/token_hash/parse_role | `crates/core/src/identity.rs` 单测 3 项 |
| platform_users 表 CRUD + 唯一约束 | `crates/store/src/libsql_store/users.rs` CRUD 测试 |
| v23→v24 迁移后 schema 版本断言更新（24） | store 各迁移测试（brain_store、display_text、inputs_recorded、project_store、schema_bootstrap、schema_v4_migration、store_migrations/{early,middle,sessions,catalog}） |
| seed token 识别、哈希反查、401、exempt | `crates/web/src/auth_mw.rs` 单测 5 项 |
| 角色权限矩阵 | `crates/control/src/role_gate.rs` 单测 3 项 |
| /api/me、用户 CRUD、自删/末位 admin 保护、吊销即失效 | `crates/control/tests/e2e/users_api.rs`：`me_reports_the_seed_admin_and_rejects_missing_tokens`、`admin_creates_lists_and_revokes_users`、`delete_protections_cover_self_and_the_last_admin` |
| 非 admin 读 + operator-only 提交/命令、agent 403、503 fail-closed | `users_api.rs`：`non_admins_get_the_read_and_operator_launch_profile`、`non_admins_submit_and_command_operator_executions_only`、`operator_kind_without_an_eligible_node_fails_closed` |
| nodes catalog kinds 含 operator | `crates/control/tests/e2e/fleet_maintenance.rs::nodes_catalog_lists_the_connected_node` |
| worker 宿主机 Operator 负载（标题/前导/完成态） | `crates/worker/tests/platform/workloads.rs::operator_executions_run_the_host_process_agent_loop` |
| SPA token 登录探测 /api/me | `crates/web/spa/src/login.dom.test.jsx` |
| operator 记录跨节点重启恢复（P1） | `crates/worker/tests/durable_execution/main.rs::operator_executions_survive_node_restart` |
| seed token 轮换重指 / last-admin 原子删除 | `crates/control/src/bootstrap.rs::seed_admin_tests`、`crates/store/src/libsql_store/users.rs::tests::{guarded_delete_never_removes_the_last_admin, token_hash_rotation_repoints_and_keeps_uniqueness}` |
| SPA 刷新后身份重探（401 回弹窗） | `crates/web/spa/src/app.dom.test.jsx::re-probes_/api/me_on_refresh_and_restores_the_admin_identity` 等 |
| 用户管理抽屉（列表/建号/一次性 token/删除/保护提示） | `crates/web/spa/src/admin/usersDrawer.dom.test.jsx` |
| Operator 子 tab（admin 可见、节点表、发起弹窗） | `crates/web/spa/src/operators/{panel,launchModal}.dom.test.jsx` |
| 导航角色过滤与 admin 入口 | `crates/web/spa/src/nav.test.js`、`app.dom.test.jsx` |
| bash 工具后代清理断言对容器僵尸进程稳健 | `crates/session/src/tools/bash.rs::dropping_tool_future_kills_descendants_and_unregisters`（预存缺陷修复） |

## 评审修复（同迭代，代码审查后追加）

- **P1 journal 重启扫描补齐 Operator**：`worker::layout::ALL_KINDS` 加入 `ExecutionKind::Operator`（7→8）。此前 operator 记录写入 `node/operator/` 但 `load_current` 不扫描该目录——节点重启即静默丢失：不恢复、排队任务永不启动、控制面 inspect 404、pending 计数低估。回归测试：`worker/tests/durable_execution/main.rs::operator_executions_survive_node_restart`（完成态可寻址 + 排队任务重启后由调度器续跑 + 中断/取消记录仍在，回退该修复即红）。
- **SPA 刷新/链接登录后重探身份**：`main.jsx` App mount 时对已存 token 探测 `GET /api/me`（成功 `setIdentity`，401 经 api.js `clearToken` 回登录弹窗）。此前 identity 仅交互登录时探测一次，刷新后非 admin 看到全量导航、admin 丢失 IdentityBadge/后台管理/Operator 页签。测试：`app.dom.test.jsx::re-probes_/api/me_on_refresh_and_restores_the_admin_identity`、`drops_a_stored_token_the_refresh_probe_rejects_401_back_to_the_login_modal`。
- **seed token 轮换语义**：`bootstrap::seed_admin` 撞名且既有 `admin` 行为 admin 角色时，改为把该行 digest 重指到新启动 token（`Store::update_user_token_hash`，LibsqlStore 落地）——轮换启动 token 即吊销旧 seed 凭证；非 admin 角色的 `admin` 命名行保持跳过不覆盖。测试：`bootstrap::seed_admin_tests`（幂等/轮换重指/非 admin 行不动）。
- **last-admin 删除 TOCTOU 原子化**：新增 `Store::delete_user_guarding_last_admin`（`GuardedDelete::{Deleted,Missing,LastAdmin}`），admin 计数守卫与 DELETE 同语句执行，两个 admin 并发互删不可能双双通过；`DELETE /api/users/:name` handler 改用该原子路径。测试：`libsql_store::users::tests::guarded_delete_never_removes_the_last_admin`、既有 e2e 保护用例不变。
- P3 清理：token 注释/单测名改为「两个 ULID（各 80 bit 随机）」；`users_api.rs` 断言钉住 `oc_` 前缀；`agentsConfig.jsx` Operator 页签注释改为「仅 admin 显示入口，后端允许 user/root 提交」。
- 文档知情项：executions/messages 为平台级只读（无按用户行级隔离，简报既定设计）；允许创建名为 `admin` 的表用户（seed 行因自删检查无法删除它，方向安全）；**operator 是向非 admin token 开放的宿主机未沙箱执行通道——发放 user 角色 token ≈ 发放节点宿主机 agent 权限**，运维发放凭证时须知。

## 验证结果

- 全量 `cargo test --workspace --locked --no-fail-fast`：**4986 passed / 0 failed / 0 ignored**（361 个测试目标；上轮 4985+1 项环境性失败为预存 bash 僵尸进程误判，见上表末行，修复后 4/4 稳定通过）。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告；`cargo build --workspace --locked`：通过。
- SPA：全量 **541 passed / 67 文件**；`scripts/build-spa.sh` 重建后 `scripts/check-spa-drift.sh` 无漂移。
- 行数与敏感信息检查：新增文件 ≤400 行（最大 `users_api.rs` 243 行）；token 仅创建响应中一次性返回，仓库无硬编码凭据（e2e 使用既有 `e2e-bearer-token` 测试常量）。
