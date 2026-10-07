# 项目存储统一 libsql 与 CI 修复

## 范围与原因

Server 和 Node 的实际数据链路使用 libsql。旧独立 Web 仍保留可选 MySQL/StarRocks 项目后端，另有 SQL 专用 CI，因此工作流失败不能解释为线上改用了 MySQL。按用户决定删除这套可选实现、配置项、工厂及专用测试，项目与会话复用同一个 libsql 实例；不新增表或产品环境变量，不迁移现有业务数据。

Brain CI 原先没有准备原生 DAG 镜像和完整 NFS、容器运行条件。本地复现确认原生重启及浏览器测试依赖这些条件；旧远端失败步骤的完整日志未取得，不将本地复现当作旧远端错误的逐行还原。

## 实现

- 删除可选 SQL 后端及 sqlx，锁文件减少 41 个包，没有新增或升级依赖；已有 libsql 项目、事务和迁移测试继续保留。
- `project-store-tests.yml` 使用真实 libsql 测试；全工作区格式检查与 Clippy 保留，失败日志上传为 artifact。
- Brain CI 分开准备镜像、项目恢复、里程碑、调度、重启与浏览器步骤。构建使用普通执行用户，原生测试进入独立挂载和 PID 空间，退出时清理该空间内的残留子进程。
- 检查三个 runner 文件齐备，并比较支持 `--build-info` 的 DAG/步骤 runner；实际容器 fixture 继续验证测试程序与镜像的版本。零测试或忽略测试不能计为该阶段验收通过。
- 使用独立工作区合入已部署的 `d5780396`，保留项目 schema 33、原生执行引用、维护发布和 Codex 路径修复。共享工作区内其他任务尚未提交的改动未纳入本次交付。

## 测试覆盖

| 功能 | 测试或入口 | 文件 |
| --- | --- | --- |
| 关闭后重开保留项目、标签、看板、执行引用与运行结果；无效标签修改回滚 | `reopening_libsql_preserves_catalog_board_and_execution_results` | `crates/store/tests/project_store/reopen.rs` |
| libsql 项目数据、并发领取与事务 | `cargo test --locked -p opencoder-store` | `crates/store/tests/` |
| 失败输出和退出码、缺镜像报错、零用例拒绝、原生权限参数 | `BrainRunnerTests` | `scripts/ci/test_brain.py` |
| 项目恢复、里程碑、调度、Server 重启、浏览器执行详情 | `scripts/ci/brain.py` 各独立阶段 | `.github/workflows/brain-e2e.yml` |
| 全站交互、四种屏宽、TUI 和真实原生执行 | 全站 UI 验收入口 | `scripts/acceptance/ui/main.js` |
| 运行中任务、滚动发布、回滚及持续受理 | 平滑发布和线上信号验收 | `scripts/acceptance/smooth_release/` |

## 验证

证据目录：`/var/tmp/opencoder-libsql-ci-20261007`。

- `cargo test --locked --workspace`：5686 passed、0 failed；7 个默认忽略的用例随后显式通过（6 个原生测试与 1 个浏览器测试）。
- `cargo clippy --locked --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、全工作区构建通过。
- libsql Store：307 项通过；SPA：980 项通过，构建通过；CI 辅助脚本：5 项通过；发布相关 Python 回归：203 项通过。
- Brain CI 各阶段本机执行：project 54、milestone 14、scheduler 12、restart 2、browser 1，均通过，无忽略项。
- 全站 UI：15 组全部通过，覆盖四种屏宽、平台/项目/看板/DAG/TODO/Brain/Ontology 功能与 TUI。

- 提交前真实平滑切换与回滚验收通过，15 分钟观察完成 57 个任务；切换阶段 93 次提交无失败，最大受理延迟 0.389 秒、最大调度间隙 0.799 秒，按最大值 30 秒验收。证据：`precommit-smooth/opencoder-smooth-4nx0736l/result.json`。

删除的测试仅属于用户决定移除的可选 SQL 后端及配置、工厂；libsql 现有业务与迁移回归保持覆盖。
