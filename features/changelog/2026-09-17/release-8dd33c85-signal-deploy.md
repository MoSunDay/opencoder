Commit: 8dd33c85dc9f3c80416168e41b90b1fab8265867

# 信号发布 rel-8dd33c85（SPA 定时任务页 + TUI 压缩 Done 帧）

2026-09-17 信号发布上线 `rel-8dd33c85dc9f3c80416168e41b90b1fab8265867`（前序 `rel-4676663c`），`scripts/platform/deploy.sh --signal --bundle /srv/releases/rel-8dd33c85 --wait-seconds 300`，成功 attempt `fd50bc9a55b34a84bf95670258c7809f`，`phase=complete`。本批特性：[spa-schedules-page](spa-schedules-page.md)、[tui-compact-done-frame](tui-compact-done-frame.md)（含 `8dd33c85` 测试绑定编译修正与测试清单补记）。

## 发布路径

- 主工作区存在并行会话在途改动（`crates/tui` app.rs 等），沿用 worktree 解耦惯例：`git worktree add --detach /tmp/oc-release-8dd33c85 8dd33c85` 干净检出构建发布。
- 回归/构建独立 `CARGO_TARGET_DIR=/tmp/oc-release-target-8dd33c85`；SPA 测试复用主工作区 node_modules 软链，测试后移除。
- bundle `/srv/releases/rel-8dd33c85`（manifest commit `8dd33c85` 校验一致）。

## 发布事故与恢复（/usr/local/bin launcher 契约破坏）

- 21:06 某并行进程将 `9f9f7575`（另一开发线的 TUI 修复提交，`spa_sha256=unknown` 非 release 构建）的裸 ELF 写入 `/usr/local/bin/opencoder`，破坏「launcher 必须指向 `.opencoder-platform-current/bin/*`」的平台不变量（其余三个 launcher 均为正常 symlink）。
- 首次 deploy 在安装 `rel-8dd33c85` 时被 install_bundle fail-closed 拒绝（`existing launcher differs from current platform: /usr/local/bin/opencoder`），随后为 origin 重装基线同样 exit 4；journal 进入 `rolled_back`。
- 处置：裸二进制备份为 `/usr/local/bin/opencoder.bak.manual-9f9f7575-202609172106` 后恢复 symlink；第二次触发因 journal 残留 `rolled_back` 被 runner 判定「未到达目标」空转返回（预期语义，未触碰平台状态）；第三次 attempt `fd50bc9a` 一次通过。

## 上线复核

- `/api/health`：`{"commit":"0.1.0 (8dd33c85)","ok":true,"protocol_version":10,"role":"control"}`。
- Server/Runtime/Host unit `opencoder-server-rel-8dd33c85…` / `opencoder-runtime-rel-8dd33c85…` / `opencoder-host-rel-8dd33c85…` 均 active；`.opencoder-platform-current` → `8dd33c85…`。
- SPA 新页生效：在线 `/static/app.js` sha256 与发布 worktree `crates/web/spa/dist/static/app.js` 一致（`9f55a1bf…`）；后端契约 `GET /api/schedules` → 200。

## 回归门证据（发布点 8dd33c85 全量）

- `cargo clippy --workspace --all-targets -- -D warnings` 零警告；`cargo build --workspace --bins` 零错误；`cargo test --workspace -- --test-threads=4` 5309 passed / 0 failed / 7 ignored（403+ 二进制，含 operator/dag/todos/team/brain e2e 套件；e2e 需先 `cargo build --workspace --bins` 构建 split binaries）。日志 `/var/tmp/opencoder-release-20260917/workspace-test3.log`。
- SPA：`npm run test` → 114 文件 / 838 passed / 0 failed；`check-spa-drift.sh` 零漂移。

## 已知事项

- 并行会话在主工作区仍有在途改动（`crates/tui` 模式切换签名重构等），按惯例待其落地后整批复验再发；本回执只对已上线的 rel-8dd33c85 负责。
- 遗留备份：`/usr/local/bin/opencoder.bak.manual-9f9f7575-202609172106`（9f9f7575 裸二进制备份，待并行线确认后处置）。
