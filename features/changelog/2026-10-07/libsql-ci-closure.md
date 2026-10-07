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

## 远端浏览器错误记录

`4586e9a0` 的 libsql CI 已通过；Brain 的准备、项目恢复、里程碑、调度和 Server 重启阶段均通过，浏览器失败后截图再次报 `Target crashed`，遮住了原始异常。补充错误记录，原始异常先写控制台，截图/HTML 失败分别记录，Chromium 进程日志保留；不重试测试、不跳过断言。

- `failure.test.js` 的两个用例验证页面已崩溃时仍保留原始错误，以及页面可读时保存截图和 HTML。
- Python CI 辅助脚本 5 项通过；带进程日志的真实浏览器验收通过。此次后续改动仅涉及验收脚本和工作流，Rust、SPA、依赖及产品逻辑与已完成全量回归的 `4586e9a0` 相同。

后续远端记录已确认初始 `page.goto` 发生 renderer crash；将 Chromium 的致命错误、原始异常和资源状态放在摘要前部，避免被 Rust 回溯截断。保留完整进程日志，失败时也保存依赖缓存。错误摘要回归新增 1 项（Python 合计 6 项），两个浏览器证据测试及真实浏览器验收通过。

远端 `98661440` 的错误摘要显示首次导航时 Chromium renderer 崩溃，伴随 Vulkan/SwANGLE/EGL 初始化失败；可用内存约 14 GB、cgroup OOM 计数为 0。DOM 浏览器验收关闭 GPU 加速，继续执行完整页面和真实任务断言，并重跑远端验证确认。进程错误摘要保留最后几条错误，避免重复的初始化错误遮住最终原因。

关闭 GPU 加速后，本机真实浏览器用例通过（102.64 秒）；Python CI 回归 6 项、浏览器证据回归 2 项通过。

## 已复现的浏览器根因与修复

远端 `c4c54451` 继续失败，最后的进程错误明确指向 `platform_shared_memory_region_posix.cc`：普通 runner 创建的 `browser-tmp` 不允许 Chromium 子进程写入。原生测试经 sudo 以 root 运行，Chromium 子进程会丢弃特权能力，不能靠 root 身份绕过 runner 所有的 0755 目录权限。Vulkan/EGL 初始化错误是伴随现象，关闭 GPU 并未解决问题，因此撤去该参数。

本机用同一 Chromium、同一 PID/挂载隔离和同一个 0755 目录验证：目录属普通用户时原样复现共享内存 `Permission denied`，仅把归属改为 root 后正常加载并读取页面（证据 `runner-owned-browser-temp.log` / `root-owned-browser-temp.log`）。原生阶段开始前将专属临时目录归给 root，结束后通过 finally 递归归还调用用户，保证失败证据也可上传；不放宽为全员可写。新增失败路径归还所有权回归，Python CI 测试共 7 项通过。
