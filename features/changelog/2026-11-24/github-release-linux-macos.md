# GitHub Release：Linux x86_64 / macOS aarch64 平台包自动构建与发布

一句话：新增 `.github/workflows/release.yml`，`v*` tag 在 `ubuntu-latest`（linux-x86_64）与
`macos-latest`（macos-aarch64）上运行 `scripts/platform/release/build.sh` 产出带
`manifest.json` / `SHA256SUMS` 的平台包，汇总校验后发布 GitHub Release；workspace 版本
0.1.0 → 0.1.1，并重建 SPA dist 消除发布构建的 drift 阻断。

## 改动

- `.github/workflows/release.yml`：`push` tag `v*` 触发（另留 `workflow_dispatch`）；
  `bundle` 矩阵作业各自构建平台包、打 `opencoder-<version>-<platform>.tar.gz` 并生成
  `.sha256`；`release` 作业仅在 tag 触发时运行，`sha256sum -c` 校验全部产物后用
  `gh release create --verify-tag --notes-file` 发布（release 已存在时改走
  `gh release upload --clobber`）；发布失败时日志尾部写入 step summary 与 error annotation。
- 工作流加固：Linux 安装 `libclang-dev cmake pkg-config`（bindgen 依赖）、macOS 安装
  cmake；`setup-node` 固定 Node 24 与提交 dist 的构建环境一致；`uname -m` 校验平台标签；
  tag 与 workspace 版本一致性校验；`COPYFILE_DISABLE=1` 避免 macOS AppleDouble 条目；
  `GH_REPO` 显式指定发布仓库；工作流级 `contents: read`，仅 release 作业 `contents: write`；
  checkout `persist-credentials: false`；同 tag 重推由 `cancel-in-progress: true` 取代旧运行；
  构建失败时把日志尾部写入 step summary 与 error annotation，便于匿名读取失败原因。
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

## 发布执行记录（2026-11-25）

- 触发：tag `v0.1.1` → Release 工作流
  [run 45510608532](https://github.com/MoSunDay/opencoder/actions/runs/45510608532)：
  `bundle (linux-x86_64)`、`bundle (macos-aarch64)` 与 `publish github release` 均 success。
- Release：<https://github.com/MoSunDay/opencoder/releases/tag/v0.1.1>
  - `opencoder-0.1.1-linux-x86_64.tar.gz`（91,709,505 字节，sha256
    `9e687c6cfc5e4e141a7e34053c74877d228d4826642bac8f760e20c0a47dd640`）+ `.sha256`
  - `opencoder-0.1.1-macos-aarch64.tar.gz`（40,505,998 字节，sha256
    `f0bb32ac0564f20fba1084039855d7c651016869f5408297909033e9f5e25beb`）+ `.sha256`
- 下载核验：两个归档 `sha256sum -c` → OK；包内 `SHA256SUMS` 逐项 → OK。
- Linux 包：7 个二进制（opencoder / opencoder-cli / opencoder-server / opencoder-agent /
  dag-runner / agent-step-runner / agent-session-runner）；`opencoder --version` =
  `0.1.1 (844df6f0)`，`--build-info` 的 `git_dirty=false`、`commit 844df6f09…`、
  `spa_sha256` 与 dist 树一致；在 glibc 2.35 主机运行仅有 `GLIBC_2.39` weak version
  警告（二进制构建于 ubuntu-24.04）。
- macOS 包：3 个二进制均为 `Mach-O 64-bit arm64 executable`；本机为 Linux 无法执行，
  `manifest.json` 与 `SHA256SUMS` 已核验（commit / version / spa_sha256 与 Linux 包一致）。
- 期间修正：run 2 的 Linux 构建失败（加固原生依赖与 Node 版本后消失）；run 3 两个平台
  构建成功但 `gh release create` 1 秒内失败，改为 `--notes-file` 并把失败日志写入
  step summary / error annotation 后 run 4 成功。

## 收尾修正（2026-11-25）

- `.github/workflows/release.yml`：tag 与 workspace 版本一致性校验前移为构建前的独立 step
  （原先位于构建之后的归档 step，tag 打错时两个 runner 都会白跑完整构建）；归档 step 仅保留
  版本提取。
- `.github/workflows/release.yml`：Release 说明追加 tag 对应的 `features/changelog/` 目录链接，
  便于从发布页找到「功能 → 测试名」映射。
- `README.md` / `README.en.md`：补充 Linux 包的 glibc 基线（ubuntu-24.04 / glibc 2.39 构建，
  glibc 2.35 主机验证二进制可启动（`--version` / `--build-info`），更低版本未验证）。
- 溯源说明：tag `v0.1.1` 指向 `844df6f09`，该提交不在 main 祖先链上（main 历史在并行开发期间
  已变更，原因未追溯），其 tree 与 `f81f64ca0` 相同（`2cfab4b0c`）；发布内容与 `--build-info`
  的 commit 以 tag 为准。已发布的 v0.1.1 Release 说明未含 changelog 链接（本机无发布凭据，
  无法回填），新说明模板自下一次 tag 起生效。
- 措辞收敛：README 的「验证可运行」改为「验证二进制可启动（`--version` / `--build-info`）」，
  溯源说明只陈述可验证事实。版本提取 `awk '/^version = "/'` 在虚拟 manifest 下正确，且 tag
  校验使用同一提取，提取错误会在构建前失败而非误发布，故保持不改。

## 测试覆盖（收尾修正）

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 工作流 YAML 语法 | `python3 -c "yaml.safe_load(open('.github/workflows/release.yml'))"` | `.github/workflows/release.yml` |
| 工作流 `run` 脚本语法 | 提取全部 `run:` 块（`${{ }}` 替换为占位符）后逐块 `bash -n` | `.github/workflows/release.yml` |
| 版本校验 step | 以 `GITHUB_REF_NAME=v0.1.1`（通过）/ `v0.1.2`（拒绝）/ `main` 分支（跳过）模拟执行 step 脚本 | `.github/workflows/release.yml` |
| Release 说明模板 | 以 `GITHUB_REF_NAME=v0.1.1` 渲染 heredoc，断言含 changelog 链接 | `.github/workflows/release.yml` |

- 定向检查：YAML 解析 → OK；`bash -n` 全部 `run` 块 → 0 失败；版本 step 三种输入 → exit 0 / 1 / 0
  符合预期；说明模板渲染 → 链接正确；归档 step 不再含重复校验。
- 文档：README 与 changelog 仅检查内容与链接；工作流改动未在 GitHub 上实际触发（下一次推 tag 时
  生效）。未运行全量测试。
