# GitHub Release：Linux x86_64 / macOS aarch64 平台包自动构建与发布

一句话：新增 `.github/workflows/release.yml`，`v*` tag 在 `ubuntu-latest`（linux-x86_64）与
`macos-latest`（macos-aarch64）上运行 `scripts/platform/release/build.sh` 产出带
`manifest.json` / `SHA256SUMS` 的平台包，汇总校验后发布 GitHub Release；workspace 版本
0.1.0 → 0.1.1，并重建 SPA dist 消除发布构建的 drift 阻断。

## 改动

- `.github/workflows/release.yml`：`push` tag `v*` 触发（另留 `workflow_dispatch`）；
  `bundle` 矩阵作业各自构建平台包、打 `opencoder-<version>-<platform>.tar.gz` 并生成
  `.sha256`；`release` 作业仅在 tag 触发时运行，`sha256sum -c` 校验全部产物后用
  `gh release create --verify-tag --generate-release-notes --clobber` 发布。
- 版本：`Cargo.toml` `[workspace.package] version = "0.1.1"`，`cargo update --workspace`
  同步 26 个 workspace 成员的 lock 版本；`README.md` / `README.en.md` 徽章与对比表同步。
- SPA dist：HEAD 的 `crates/web/spa/dist/static/app.js` 与 `src/` 漂移，
  `scripts/build-spa.sh` 重建后 `check-spa-drift.sh` → `spa dist: no drift`
  （4,508,460 → 4,508,857 字节），解除 `build.sh` 的 drift 阻断。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 版本格式与编译期元数据 | `format_version_clean`、`format_version_dirty_appends_marker`、`format_version_unknown_commit_is_honest`、`baked_long_version_matches_format_contract` | `crates/core/src/version.rs` |
| 发布元数据构建契约 | `test_release_commit_refreshes_shared_worktree_cache_and_rejects_mismatch` | `scripts/platform/release/test_build_metadata.py` |
| 平台包构建（Linux） | `scripts/platform/release/build.sh --output ...` | `scripts/platform/release/build.sh` |
| 平台包安装 | `scripts/install.sh --bundle ... --dest-dir ...` | `scripts/install.sh` |
| SPA dist 无漂移 | `scripts/check-spa-drift.sh` | `scripts/check-spa-drift.sh` |
| 工作流 YAML 语法 | `python3 -c "yaml.safe_load(open('.github/workflows/release.yml'))"` | `.github/workflows/release.yml` |

- 定向回归：`cargo test -p opencoder-core --lib version` → 12 passed / 0 failed。
- 定向回归：`python3 scripts/platform/release/test_build_metadata.py` → 1 test OK。
- 构建：`scripts/platform/release/build.sh --output /root/opencoder/dist/opencoder-0.1.1-linux-x86_64`
  → 15m48s；7 个二进制（opencoder / opencoder-cli / opencoder-server / opencoder-agent /
  dag-runner / agent-step-runner / agent-session-runner）+ `manifest.json` + `SHA256SUMS`，
  包内 `sha256sum -c` 全 OK；`--build-info` → `version 0.1.1`、`git_dirty=false`、
  `commit dd858ea16`、`spa_sha256` 与 dist 树一致。
- 安装：`scripts/install.sh --bundle ... --dest-dir /tmp/oc-install-test` → exit 0，
  安装后 `opencoder --version` = `0.1.1 (dd858ea16)`。
- 归档：`tar -czf` 后 `sha256sum -c` → OK，包内含 `manifest.json` / `SHA256SUMS` / `bin/`。
- macOS aarch64：本机为 Linux，无法本地编译；由 tag 触发的 GitHub Actions
  `macos-latest` 作业构建，结果在发布页产物与工作流运行记录中核验。
- 测试范围：本次改动及其直接影响链路；未运行全量测试。
