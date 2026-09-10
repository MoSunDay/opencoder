# 平台新版本部署 fd9bd470（b465f440 → fd9bd470，protocol 7 → 8）

## 部署内容

- 构建 `dist/opencoder-platform-fd9bd470`（manifest 校验 + SHA256SUMS 通过；`protocol_version: 8`，含 Responses API、DAG run 步级查询等）。
- `scripts/platform/install_bundle.py --backup` 原子激活到 `/usr/local/bin`；回滚点 `.opencoder-platform-rollbacks/1789048942439059081-b465f440381b`。
- 手工 nohup 进程无法跨工具会话存活，fleet 改为 systemd 托管：`/etc/systemd/system/opencoder-server.service`（:18080）+ `opencoder-agent@.service`（node02/node04 模板实例）。
- 滚动重启顺序：server → agent；`DELETE /api/admin/drain` 重开上次发布遗留的 admission freeze。
- 验证：`/api/health` → `0.1.0 (fd9bd470)` / protocol 8；node02、node04 online+ready+idle，`resource_error: null`。

## 部署前修复

- `fix(tui): notepad 视口测试注入 size_override`（fd9bd470）：`editor_viewport` 经 crossterm 回退读宿主 `/dev/tty`（41 行）导致 `notepad_scroll` 测试随机器尺寸漂移；新增 `NotepadView::size_override` 注入口，测试固定 80×24。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 视口注入消除 /dev/tty 依赖 | `j_advances_scroll_incrementally`、`big_g_advances_scroll`、`gg_resets_scroll_to_zero`、`ctrl_d_moves_cursor_down`、`ctrl_u_moves_cursor_up`、`ctrl_f_full_page_down`、`page_scroll_does_not_fire_in_insert_mode` | `crates/tui/tests/notepad_scroll.rs` |
| tui 包回归 | `cargo test -p opencoder-tui` → 1795 passed / 0 failed | — |
| 发布包完整性 | build.sh 内建 manifest/SHA256SUMS 校验 + install_bundle.py 安装前校验 | `scripts/platform/release/build.sh` |

- clippy：`cargo clippy -p opencoder-tui --all-targets -- -D warnings` → 零警告
- 注：应用户要求本次跳过全量 `cargo test --workspace`，仅运行了 tui 包全量测试；上一轮全量回归除上述环境依赖测试外全部通过（4592 passed / 1 env-dependent failed）
