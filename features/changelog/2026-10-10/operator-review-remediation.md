Commit: 42456123489c6fbb5f108a575e249d48de59d5eb

# Operator 与发布前审查修复

## 行为

- 发布数据格式升为 7，阻止旧格式与 schema v36 候选直接滚动共写；维护预检及恢复覆盖人员关系迁移，保持已有鉴权数据的严格一致性检查。
- 发布项目验收将已读取的要求、归属、能力、执行与依赖作为提交前提，在同一事务完成核验与看板修改；过期前提返回 409，不覆盖并发修改。
- Operator 派生会话保留类型，复制父配置、冻结 skills 和私有 Codex 状态，使用新的 HOME 与空 workspace；普通 Agent 不注入 Operator 默认规则。
- Ontology 验收选择系统临时连接范围之外的独立监听端口，避免服务启动前端口被占用。

## 功能 → 测试

| 功能 | 测试入口 |
| --- | --- |
| 发布格式隔离 | `project_people_schema_requires_maintenance_from_previous_formats`；`scripts/platform/rolling_tests/test_manifest.py` |
| 迁移预检与可重复恢复 | `scripts/platform/maintenance_tests/test_preflight.py`、`test_project_people.py` |
| 并发验收与重试 | `crates/control/tests/e2e/project_api/acceptance.rs` 三项 HTTP 用例 |
| 派生后的类型、规则、HOME、技能与重启 | `remote_forks_preserve_conversation_type_and_policy_after_restart`、`fork_copies_frozen_state_without_sharing_writes` |
| 原生执行与完整界面 | `tests/dag_e2e/main.rs`、Worker `platform`、`scripts/acceptance/ui/main.js` |

## 验证范围与限制

以固定的 schema v36 源码快照验证，保留工作区并行修改。Rust workspace 5,784 项通过、8 项原有忽略；SPA 166 个文件、1,157 项通过；Python 回归 228 项通过。构建、Clippy、格式检查及完整 UI 的 17 项检查通过，最终镜像另通过 19 项原生执行场景。Ontology 首轮端口冲突保留失败日志，修正监听端口选择后复验通过。

相同最终构建在空闲盘完成真实切换、回滚及 900 秒观察，57 个观察任务通过。部署盘上的两次演练未通过：第三次切换的任务排队超时，以及候选探测在步骤完成后等待 overlay 卸载；内核栈显示等待底层文件系统写回。空闲盘结果不能替代部署盘的 30 秒延迟门槛与 15 分钟观察。

未部署生产，未完成正式干净发布包验收。验证期间新增的项目摘要与 schema v37 并行修改不在上述快照内，不能据此宣称当前整个工作区可上线。

## 相关索引

[Worker](../../../agents/worker/index.md)、[Store](../../../agents/store/index.md)、[项目](../../project/index.md)、[平台](../../agent-platform/index.md)。
