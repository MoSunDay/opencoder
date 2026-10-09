Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# 全站 UI 与原生 DAG 验收闭环

## 范围与行为

- 保留现有视觉风格，覆盖既有 10 个 Web 导航页与连接 Server 的 TUI。
- DAG 页提供原生二进制上传、不可变版本、下载、当前版本切换和删除；步骤编辑器区分受理时 current 与明确版本。
- DAG 与执行明细使用节点保存的只读运行环境，展示共享容器、步骤目录与实际固定摘要，不查询当前池来推测历史。
- Agent 配置展示三个实际 NFS 导出；身份、资源、调度配置和触发历史读取失败明确报错，取消旧读取并防止迟到响应覆盖。
- 定时任务历史打开原执行；窄屏 Team 工具栏可换行。全站验收同时检查导航范围、构建配套、真实功能、四种屏宽和 TUI。

## 功能与测试

| 功能 | 测试或验收入口 | 文件 |
| --- | --- | --- |
| ELF、版本引用与上传边界 | `accepts Linux ELF64 architectures and preserves uploaded bytes`、`rejects empty, oversized, non-ELF and incompatible executables` | `crates/web/spa/src/dag/resources/model.test.js` |
| 资源失败与确认操作 | `shows actual versions and confirms pointer-only switches`、`requires confirmation before deleting all versions`、`keeps upload input and error visible when save fails` | `crates/web/spa/src/dag/resources/resources.dom.test.jsx` |
| 身份门控与错误重试 | `does not render privileged navigation before identity confirmation`、`rejects malformed identity responses rather than waiting forever` | `crates/web/spa/src/app.dom.test.jsx` |
| 请求取消与迟到响应 | `aborts replaced requests and ignores late responses even if transport ignores abort`、`aborts pending requests on unmount` | `crates/web/spa/src/ui/requests/query.dom.test.jsx` |
| 实际 NFS 状态与源路径 | `does not present a failed read as a stopped export`、`shows the configured workspace source without changing it` | `crates/web/spa/src/agentNfsCard.dom.test.jsx` |
| 节点配置失败不能保存默认值 | `does not save invented settings when the node configuration cannot be read` | `crates/web/spa/src/fleet/settings/scheduling.dom.test.jsx` |
| 历史打开原执行及读取重试 | `opens the recorded execution rather than launching another task`、`keeps a failed history read distinct from no triggers and allows retry` | `crates/web/spa/src/schedule/history.dom.test.jsx` |
| 固定资源快照与损坏拒绝 | `projects_saved_versions_without_host_paths_or_current_pool_reads`、`preparation_does_not_invent_resource_versions`、`damaged_or_mismatched_snapshots_are_rejected` | `crates/worker/src/operations/query/dag_context.rs` |
| 实际上传、版本固定、共享文件与源不变 | `platform` 验收的 resources、exports、schedules、teams、chat 场景 | `scripts/acceptance/ui/scenarios/`、`scripts/acceptance/platform.js` |
| 项目、TODO、动态 DAG、日志产物、Brain 与 TUI | `CASES` 注册的完整功能矩阵 | `scripts/acceptance/ui/scope.js` |
| 全页面与弹窗四种屏宽 | `responsive-1920`、`responsive-1280`、`responsive-768`、`responsive-390` | `scripts/acceptance/spa_responsive.js` |
| 导航遗漏与未知范围拒绝 | `verifyCoverage` 测试 | `scripts/acceptance/ui/scope.test.js` |

## 验证结果

- SPA 全量：122 个测试文件，928 项通过；SPA 构建和产物漂移检查通过。
- Rust 全量：`cargo test --locked --workspace --no-fail-fast -- --test-threads=2`，436 组结果，5593 passed / 0 failed / 7 ignored。7 项需真实环境的手动用例另行全部通过：3 项 NFS、3 项 runc 和 1 项 Chromium。
- `cargo fmt --all -- --check` 与 `cargo clippy --locked --workspace --all-targets -- -D warnings` 通过；发布相关 Python 回归 148 项和验收辅助 Node 测试 7 项通过。
- 全站入口 14 项检查通过，覆盖 10 个页面；四种屏宽共 104 次测量，无页面溢出、未知夹具请求或浏览器异常。回执：`/data00/opencoder-ui-closure/global-final-5/receipt.json`。
- 真实双节点 runc：20 个基础样本，900.001 秒观察、78 个观察样本，覆盖固定版本恢复、节点重启、3 个动态实例和 16 MiB 产物；4 个失败步骤及 1 个取消是预期故障分支。源不变、无剩余挂载、服务退出均为 0。回执：`/root/.cache/opencoder-e2e/20261001-ui-closure-final/evidence/result.json`。
- 平滑切换及回滚：16 个场景、71 次提交、零失败；最大受理 2.305 秒、受理间隔 2.405 秒、调度间隔 2.811 秒、调度延迟 3.250 秒，均按最大值满足 30 秒门槛。回执：`/root/.cache/opencoder-e2e/opencoder-smooth-5n6c7ngq/result.json`。

验证使用独立源码、构建目录、临时数据库与私有挂载空间。真实功能使用成套开发二进制和配套 rootfs，构建信息为基线提交的 dirty 状态，原生验收标记 `release_bundle: false`；不冒充干净提交的正式发布包，不表示已上线。共享仓库的其他并行改动不纳入本次冻结范围。

## 相关

- [UI 验收规则](../../../rules/05-ui-acceptance.md)、[DAG 执行规则](../../../rules/04-dag-execution-contract.md)
- [Web 逻辑](../../../agents/web/index.md)、[节点逻辑](../../../agents/worker/index.md)
- [DAG 能力](../../dag/index.md)、[调度平台](../../agent-platform/index.md)
