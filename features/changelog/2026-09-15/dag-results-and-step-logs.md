# DAG 结果快照与步骤日志抽屉

## 行为

- DAG 运行页与执行详情共用结果画布。进入页面直接显示当前步骤状态，已结束运行不订阅历史事件；运行中从快照水位之后订阅增量，重连时重新同步快照。
- 移除右侧步骤信息、事件列表和底部日志卡片。点击步骤打开右侧 75vw 的「实时日志」抽屉，支持步骤/全部步骤切换、搜索、自动滚动及历史分页。
- 历史日志整批加载，较早内容保留分页入口；关闭抽屉取消加载及实时订阅。日志量不影响画布状态存储，查看运行时暂停后台运行列表轮询。
- 节点通过已有生命周期事件与回执生成 `dag_steps`，返回 `head_seq` 及 running/interrupted 计数；新尝试覆盖旧回执，取消后未开始的步骤显示「未执行」。没有数据库迁移或新增配置。

## 验证

- SPA 全量：97 个测试文件、712 项通过；最终增量另覆盖取消后的未执行节点、快照衔接、抽屉切换、历史批量加载和实际滚动。
- Store / Control / Web：799 项通过。
- Worker 单元、platform 与 dag_live_logs：67 项通过；1 项依赖特权 runc 的大脑 CLI 测试按其声明跳过。
- `cargo check --workspace` 通过。
- 浏览器验收入口：`PLATFORM_BIN_DIR=<bundle>/bin node scripts/acceptance/dag_results.js`。独立 Server/Agent 使用确定性模型响应，并校验浏览器实际加载的 SPA 与仓库产物一致。

发布沿用成套 bundle、原子安装、Server → Agent 重启、原 Node ID 校验与 admission reopen 流程；不修改认证数据。
