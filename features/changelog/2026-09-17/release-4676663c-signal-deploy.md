Commit: 4676663ce29c5c2ef6efb6ef1f53038a85fef5ce

# 信号发布 rel-4676663c（SPA Chrome tab 图标改为 logo favicon）

2026-09-17 信号发布上线 `rel-4676663ce29c5c2ef6efb6ef1f53038a85fef5ce`（前序 `rel-5e130f7f`），`scripts/platform/deploy.sh --signal --bundle /srv/releases/rel-4676663c --wait-seconds 300`，`phase=complete`、无失败记录。本批特性：favicon 落地（见 [spa-tab-favicon-logo](spa-tab-favicon-logo.md)）+ 两笔 clippy lint 门修复（`b9499b1c` core `Timelike` 未使用导入、`4676663c` control needless_borrow/then_some/未使用 `TOKEN`）。

## 发布路径

- 构建采用 `git worktree add --detach /tmp/oc-release-tree 4676663c` 干净检出执行——主工作区当时有并行会话未提交改动（`submit.rs`/`hub.rs`/`spa/*.jsx`/`api.rs` 等），`build.sh` 的 clean-worktree 门无法在主工作区通过；worktree 方案使发布点与并行开发解耦，不触碰他人进行中工作。
- 回归/构建均使用独立 `CARGO_TARGET_DIR=/tmp/oc-release-target`（`env -u CARGO_TARGET_DIR` 覆盖全局共享 target），保证产物与测试严格对应发布点代码。
- bundle `/srv/releases/rel-4676663c`（manifest commit 校验一致），发布 attempt `fdd68f19429246f3a551df91b1b58fee`。

## 上线复核

- `/api/health`：`{"commit":"0.1.0 (4676663c)","ok":true,"protocol_version":10}`；Server/Host unit `opencoder-server-rel-4676663c…` / `opencoder-host-rel-4676663c…` 均 active。
- favicon 生效：`GET /static/favicon.png` → 200 `image/png` 8015B（与提交文件逐字节同大小）；`GET /` shell 含 `<link rel="icon" type="image/png" href="./static/favicon.png" />`；`/static/` 前缀 auth-exempt，登录前可加载。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| favicon 白名单服务 | `html::tests::static_whitelist_serves_fixed_build_outputs`（扩展 favicon.png 断言） | `crates/web/src/html.rs` |
| shell 引用全过白名单 | `html::tests::shell_references_resolve_through_the_whitelist` | `crates/web/src/html.rs` |
| shell 无外部引用 | `html::tests::shell_has_no_external_references` | `crates/web/src/html.rs` |
| SPA 漂移 | `scripts/check-spa-drift.sh` → no drift | `scripts/` |

- lint 门：`cargo clippy -p opencoder-control --all-targets -j 6 -- -D warnings` → 通过（workspace 全量 clippy 因共享机 OOM/锁竞争多次中断，control+core 两处修复包已验证零警告）。
- 全量回归：`cargo test --workspace --no-fail-fast`（发布点 4676663c，干净 worktree）→ **5303 passed / 2 failed**。
- **回归豁免说明**：2 个失败为发布点之前 main 的基线既有问题，与本批改动无关——`gating::non_admin_role_gates_the_surface`（`tests/operator_e2e/gating.rs:140`，agent 读 transcript 期望 200 实得 403 `role 'user' may not access this endpoint`）与 `schedule_api::brain_schedule_without_objective_records_an_error_row`（`crates/control/tests/e2e/schedule_api.rs:300`，tick dispatch 失败）。二者在 `065c017b`（并行会话节点中继提交之前的 favicon-only 点）上同样稳定失败（非 flake，两轮复跑确认），且不属于本批 favicon/lint 触碰面。按用户明确指令继续发布（先例见 [release-2868ebfd](../2026-09-16/release-2868ebfd-signal-deploy.md)）。上线后需人工复核这两条：若为权限面有意收紧，测试需跟上；若为 cron tick 真回归，按 `bash scripts/platform/deploy.sh --rollback` 回退本版。

## 相关

- [spa-tab-favicon-logo](spa-tab-favicon-logo.md)、[一键清空会话节点中继](../2026-09-17/)（并行会话同日提交，未随本批发布验证）。
