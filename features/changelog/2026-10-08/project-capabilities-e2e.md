Commit: 07d29e11815cdbbbfc82a73a5a208bd0eb313e62

# 项目、专项、TODO 与泛化能力的端到端验收

浏览器创建项目、专项和四个 TODO，分别派发 Agent、Operator、DAG 和 Brain。
Brain 第一层并行计算，第二层把实际完整输出绑定到 DAG 验证；TODO 与 Agent 页打开同一执行。
脚本核验计算结果、实时结果读取、多次指派、关联幂等、失败恢复和手动完成进度，已纳入全站 UI 验收。

修复从 TODO 直接打开 DAG 时节点列表依赖先访问节点页的问题；指派记录保留执行引用，节点离线显示读取错误。
动态 DAG 验收等待整个运行终态后再停止节点。图谱验收在改变屏宽前移开焦点并等待悬浮提示关闭。

## 测试覆盖

| 功能 | 测试名或入口 | 文件 |
|---|---|---|
| 四类派发、结果绑定、历史、进度与故障恢复 | `project-capabilities` | [main.js](../../../scripts/acceptance/project/capabilities/main.js) |
| 计算值和原生文本结果的严格校验 | `task conclusions require structured computed values, including Markdown and nested output`、`Brain child outputs accept native text or objects while checking the task result` | [scenario.test.js](../../../scripts/acceptance/project/capabilities/scenario.test.js) |
| DAG 入口独立加载节点及错误提示 | `loads dispatch nodes without first visiting the fleet page`、`shows node fetch failures in the dispatch drawer` | [dag.dom.test.jsx](../../../crates/web/spa/src/dag/dag.dom.test.jsx) |
| 全站功能、四种屏宽、TUI | `scripts/acceptance/ui/main.js` | [全站入口](../../../scripts/acceptance/ui/main.js) |

验证回执位于 `/tmp/opencoder-project-capabilities-delivery-20261007/`：

- `delivery.json` 汇总版本、文件摘要和验证结果。
- 最终隔离 E2E：`/tmp/opencoder-todo-workbench-Tw4i5i/report.json`，PASS；模型为确定性夹具，Server、Node、NFS 与 runc 为真实实现。
- SPA：147 个文件、1052 项通过；断言单测 2 项通过；验收基础设施单测 7 项通过；维护脚本测试 53 项通过。
- Clippy、格式检查、工作区构建通过；全站 UI 16 项全部通过，保留失败尝试和后续成功日志。
- 全量 `cargo test --workspace --no-fail-fast --jobs 2 -- --test-threads=4` 首轮：5695 passed、5 failed、8 个原有 ignored。失败涉及端口冲突、runc 删除、容器卸载和实时日志等待；不将首轮记成全绿。
- 五个失败目标均整组复测通过：Agents 42 项、Codex runc 1 项、实时日志 1 项、DAG 10 项、Operator 11 项。原生组使用独立挂载空间和内存临时盘，最后两组同时固定匹配的二进制与 rootfs；断言和超时条件保持原样。日志分别为 `agents-suite-retry.log`、`dag_codex_runc-isolated-retry.log`、`dag_live_logs-isolated-retry.log`、`dag_e2e-fast-retry.log`、`operator_e2e-fast-retry.log`。

相关：[项目能力与验收说明](../../../scripts/acceptance/project/README.md)、[项目能力](../../project/index.md)、[Web 逻辑](../../../agents/web/index.md)。
