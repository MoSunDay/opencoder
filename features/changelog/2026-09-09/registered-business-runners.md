# 注册业务 Runner 与归因、回归任务迁移

归因和回归原先由业务 API 自行启动 Codex。现在保留原 API、CLI、请求校验、报告地址与投递流程，由 opencoder 受理并持久化排队，通过注册的 DAG Runner 执行原业务流程。

- Agent 绑定命名 Codex 配置，模型、权限、授权槽位和环境由 Harness 管理。受理时固定配置版本、Runner 安装文件校验和及 Prompt/Skill/Tools 资源版本；修改配置不会改变已排队任务。
- Skill 资源支持完整包，入口、引用合同和资源文件一起生成版本；禁止缺入口、重复文件及路径逃逸。
- Runner 以前台 NDJSON 回传阶段及 Codex JSONL，沿用三层折叠消息解析并支持刷新重放。完成需满足进程成功、模型阶段完整和产物校验；回归准出结论独立于执行状态。
- 业务 jobId + attempt 固定关联 executionId。受理回包丢失、服务重启及报告发布失败只对账同一尝试；中断后的显式重试创建新 attempt。完成收据可重放，缺少完成收据的已开始任务不会隐式重跑。
- 回归保留 Git manifest、OverlayFS 隔离、基线/目标测试及模型复核。API 与 Node 当前部署在同一主机，沿用原 API/CLI 触发，统一使用节点并发限制和 FIFO/LIFO 队列。
- 协议升级到 7，Server 与 Node 需要一起升级。业务程序版本为 0.7.0，使用固定发布目录与私有配置；原鉴权材料和历史记录保留。

## 验证映射

| 能力 | 验证 |
| --- | --- |
| 命名 Harness 与配置作用域 | `crates/core/tests/harness_runtime.rs` |
| Runner 协议、产物完整性、重放、取消及超时 | `crates/dag-runtime/tests/runner.rs` |
| Server/Node 受理、排队、配置冻结、消息、产物及投递状态 | `crates/worker/tests/runner_dispatch.rs` |
| 完整 Skill 包上传与版本文件读取 | `crates/web/tests/web_agent_resources.rs` |
| 配置选择、绑定及独立回归结论 | `crates/web/spa/src/harness/runner.dom.test.jsx` |
| 业务桥接、丢失回包、重启、发布恢复与旧详情接口 | 业务工具 `test/service/opencoder.test.ts` |
| 真实 Git、隔离执行、双阶段模型协议与报告 | 业务工具 `test/regression/e2e/opencoder.test.ts` |

业务侧全量测试 136 项通过，浏览器测试 6 项通过；合并工作区 Web 测试 496 项通过。Rust 全仓 4,951 项通过、0 失败、5 项既有手动用例忽略，Clippy 零警告，格式检查通过。真实 Codex 验收分别生成归因 23 个产物、回归 25 个产物，回归对比基线成功、目标失败后复核为 block。独立 Server/Node 重放上述真实消息并校验所有报告，浏览器验证折叠消息与报告下载。

合并工作区同时包含独立里程碑关系、历史待办归类及编辑界面状态修复，发布验证覆盖这些变更；迁移前先停止旧执行受理并保存完整备份。
