Commit: d63498e83d66aa3be7df652b20f83df0d4da47c1

# Web 发布 d63498e8（TODO 运行画布 + 模板全宽抽屉）

## 发布内容

- 代码 commit `d63498e8`（feat(web): todo run canvas and full-width template drawers），docs commit `6a1af9d3`（changelog/memory 字段与记忆行）。
- 构建 `dist/opencoder-platform-d63498e8`（manifest/SHA256SUMS 校验通过；`protocol_version: 9` 不变，SPA digest `f1ef4af3…` 已由编译期校验绑定）。
- `scripts/platform/install_bundle.py --backup` 原子激活到 `/usr/local/bin`；回滚点 `.opencoder-platform-rollbacks/1789123100162281595-0bc5b867a766`。
- systemd 滚动重启：server → agent；`DELETE /api/admin/drain` 重开重启后遗留的 node admission freeze。

## 验证

- `/api/health` → `0.1.0 (d63498e8)` / protocol 9 / role=control。
- served `/static/app.css` 含 `todo-editor-toolbar`、`/static/app.js` 含 `oc-todo-run-node`（新 dist 特征）。
- 节点 human-os-02：online / ready / idle，`resource_error: null`。

## 测试覆盖（发布轮，隔离 worktree @ d63498e8）

| 范围 | 结果 |
|---|---|
| SPA vitest 全量 | 82 文件 / 669 passed / 0 failed |
| `scripts/check-spa-drift.sh` | no drift |
| `cargo test -p opencoder-web`（嵌入 dist 的 crate，含 HTTP/SSE 契约） | 292 passed / 0 failed |
| `cargo check --workspace`（@ 2686d40a 干净树） | 通过 |
| build.sh 内建 manifest/SHA256SUMS + build-info 校验 | OK |

注：本轮改动仅 SPA + 文档，Rust 源码零改动，故 `cargo test --workspace` 全量以 `cargo check --workspace` + web 包全量替代（0bc5b867 轮全量基线 5051 passed 未受影响）；未跑 workspace clippy（零 Rust 改动）。

## 并发工作树说明（重要）

发布工作树当时有另一会话的活跃 WIP（约 164 项改动，含 DAG step log/事件、harness 配置管理、envs 面板删除等）。为满足 build.sh 的干净树要求，曾在主工作树 `git stash push -u`（stash@{0} "preserve-concurrent-work-before-web-release-*"）；对方会话随后又写回了更新版本。**stash@{0} 保留未 pop**（pop 会用 17:52 旧快照覆盖其新工作）；如需找回被 stash 的旧改动：`git stash list` → 与工作树逐文件比对后选择性 `git checkout stash@{0} -- <path>`。

发布实际在隔离 worktree 完成：`git worktree add --detach` → 应用本侧改动 → 构建/测试 → commit `d63498e8`。分支引用 `release/web-todo-d63498e8` 指向发布 commit；main 快进（`git merge --ff-only release/web-todo-d63498e8`）待工作树 WIP 落地后执行。

## Related Docs

- [todo 运行画布](./todo-run-canvas.md)、[模板全宽抽屉](./todo-template-drawer.md)
- [web 模块](../../../agents/web/index.md)
