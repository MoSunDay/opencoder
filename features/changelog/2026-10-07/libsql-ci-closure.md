Commit: 4bb3a745544f3b3f9898447088a919da502de6e5

# 项目存储统一 libsql 与 CI 修复

## 范围与根因

Server 和 Node 的实际数据链路使用 libsql。旧独立 Web 保留了可选 MySQL/StarRocks 项目后端和 SQL 专用 CI，这不能解释为线上改用了 MySQL。按用户决定删除这套可选实现，项目与会话复用同一个 libsql 实例；不新增表或产品环境变量，不迁移现有数据。

Brain CI 原先没有准备原生 DAG 镜像和完整 NFS、容器运行条件。补齐后，远端浏览器日志进一步定位到共享内存 `Permission denied`：普通 runner 创建临时目录，原生测试却以 root 运行；Chromium 子进程丢弃特权后，既不能写入 runner 所有的目录，也不能穿过 runner 私有父目录。失败后的截图又报 `Target crashed`，遮住了首次导航的原始异常。

本机用同一 Chromium 和 PID/挂载隔离复现了两种权限失败，并验证 `/tmp` 下 root 所有的 0700 目录可以正常加载页面。Vulkan 初始化错误是伴随现象，最终修复没有保留关闭 GPU 的参数。旧远端失败步骤的完整日志未取得，不将后续复现当作旧日志的逐行还原。

## 实现与边界

- 删除可选 SQL 后端、配置项、工厂及 sqlx；锁文件减少 41 个包，没有新增或升级依赖。保留 libsql 应用、事务和迁移测试。
- [存储 CI](../../../.github/workflows/project-store-tests.yml) 执行真实 libsql 测试，保留全工作区格式检查与 Clippy。
- [Brain CI](../../../.github/workflows/brain-e2e.yml) 分开准备镜像、项目恢复、里程碑、调度、重启与浏览器步骤。普通用户编译，原生测试进入独立挂载和 PID 空间；退出时清理残留子进程。
- 原生临时目录位于 `/tmp`，归实际执行用户所有；结束后递归归还调用用户并移入证据目录。失败日志、截图、HTML 与原始浏览器异常保留并上传，零测试或忽略测试不能算通过。
- 检查三个 runner 文件齐备，并比较支持 `--build-info` 的 DAG/步骤 runner；容器 fixture 继续验证镜像与测试程序版本。
- 独立工作区合入已部署的 `d5780396`，保留 schema 33 的执行引用、维护发布和 Codex 路径修复。共享工作区内其他任务未提交的改动未纳入交付。

## 测试覆盖

| 功能 | 测试或入口 |
| --- | --- |
| 关闭重开保留项目、Tag、看板、执行引用和旧运行结果；无效 Tag 修改回滚 | [`reopening_libsql_preserves_catalog_board_and_execution_results`](../../../crates/store/tests/project_store/reopen.rs) |
| libsql 项目数据、并发领取与事务 | `cargo test --locked -p opencoder-store` |
| 原始错误、退出码、缺镜像、零用例拒绝、权限隔离与失败证据归档 | [`BrainRunnerTests`](../../../scripts/ci/test_brain.py) |
| 页面已崩溃时保存原始失败；页面可读时保存截图和 HTML | [`failure.test.js`](../../../scripts/acceptance/brain/failure.test.js) |
| 项目恢复、里程碑、调度、Server 重启、浏览器执行明细 | [`scripts/ci/brain.py`](../../../scripts/ci/brain.py) 各阶段 |
| 全站交互、四种屏宽、TUI 与真实原生执行 | [全站 UI 验收](../../../scripts/acceptance/ui/main.js) |
| 运行中任务、发布、回滚、事件流与持续受理 | [平滑发布验收](../../../scripts/acceptance/smooth_release/) |

## 回归结果

证据根目录：`/var/tmp/opencoder-libsql-ci-20261007`。

- Rust 全工作区：5686 passed、0 failed；7 个默认忽略的用例随后显式通过（6 个原生测试和 1 个浏览器测试）。格式检查、全工作区 Clippy 零警告与构建通过。
- libsql Store：307 项；SPA：980 项及构建；发布相关 Python 回归：203 项；最终 CI 辅助脚本：7 项；浏览器证据回归：2 项，均通过。
- Brain 本机各阶段：project 54、milestone 14、scheduler 12、restart 2、browser 1，均通过，无忽略项。最终临时目录方案通过真实浏览器和证据归档验证。
- 产品 Rust、SPA、依赖和发布代码从全量回归版本 `4586e9a0` 到 `4bb3a745` 未变化；中间提交仅完善 CI、浏览器证据和文档。对照记录为 `final-source-equivalence.json`。
- GitHub [libsql CI](https://github.com/MoSunDay/opencoder/actions/runs/37574884981)、[发布脚本 CI](https://github.com/MoSunDay/opencoder/actions/runs/37574885268) 与最终 [Brain CI](https://github.com/MoSunDay/opencoder/actions/runs/37579067002) 均通过。

删除的测试仅属于用户决定移除的可选 SQL 后端及配置、工厂；libsql 现有应用与迁移回归保持覆盖。

## 发布与验收

使用仓库身份 `MoSunDay <MoSunDay@users.noreply.github.com>` 推送代码版本 `4bb3a745544f3b3f9898447088a919da502de6e5`。正式包内六个二进制、SPA 与配套 runner 版本一致，工作树标记为 clean。

- 正式包全站 UI：15 组全部通过，包括 1920、1280、768、390 四种屏宽、平台、项目、看板、DAG、TODO、Brain、Ontology 和 TUI。
- 正式包独立进程验收：发布、回滚和任务连续性通过；900.000 秒观察完成 58 个任务。最大受理延迟 1.857 秒、最大调度间隙 2.557 秒。
- 线上先发布，再回滚至 `d5780396` 并再次发布；复测再次执行信号回滚和发布。新旧 Runtime、模型 shell 和原生 DAG 在切换时继续运行，依赖链按 `first` → `second` 完成，保留实际脚本输出和工具验收记录。
- 首轮线上验收等待模型最终确认超过 300 秒，保留失败回执；任务实际在信号释放后约 384 秒全部通过。复测只将模型确认等待预算设为 600 秒，实际约 411 秒完成；受理、调度、SSE、原生执行和观察门槛未放宽。
- 最终线上验收：99 次持续提交、834 次入口探测均无失败；最大受理延迟 5.935 秒、最大受理间隙 6.035 秒、最大调度间隙 7.241 秒，按最大值 30 秒检查。SSE 两次恢复耗时 0.418、0.772 秒，持久化事件核对一致。
- 最终线上观察实际 900.000 秒，133 个任务完成；Server 与 Runtime 的实际运行文件摘要均匹配正式包，资源服务进程未重启，入口 `mode=open`、3 个可用节点。

正式包与回执位于 `release-4bb3a745/`：`prepublish-verification.json`、`ui/receipt.json`、`smooth/opencoder-smooth-tv7vxn5c/result.json`、`release-live-ae4cb59709073eb2/result.json` 和 `closure.json`。首轮超时记录位于 `release-live-cfcc6a82ab758ecb/failure.txt`，未覆盖。

相关索引：[Store](../../../agents/store/index.md)、[Web](../../../agents/web/index.md)、[Brain](../../../agents/brain/index.md)、[CI 运行说明](../../../scripts/acceptance/brain/README.md)。
