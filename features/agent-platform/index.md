Commit: 5bf6f621e3722e60258592267109ba5807e74d94

# Agent 调度平台 — Server 调度、Node 执行、Web/CLI 管理

## 执行查看

- 用户从全部执行、DAG 运行等业务入口查看运行；执行列表展示创建时间、ID、类型、节点与状态，详细结果由所属节点提供。
- DAG 直接展示当前结果，运行中继续更新。完成或取消后的未执行步骤明确标示，加载与连接错误显示重试入口。
- 点击 DAG 步骤从右侧打开占视口 75% 的「实时日志」抽屉；支持切换步骤或查看全部步骤、搜索、自动滚动和历史分页。打开历史执行不会逐条回放画布，关闭日志抽屉结束日志请求。
- 执行详情继续提供产物下载；节点离线时显示错误，恢复连接后可重新查询。

## 边界

- 执行分配后固定节点，子执行与数据节点闭环
- Server 索引仅创建时间/ID/类型/节点/状态，明细按 ID 回查节点
- 节点离线时明细查询明确报错
- 节点页删除仅移除节点注册；在线节点需先停止服务，执行索引和任务数据保留
- Server 通过只读 NFS 共享 Agent 资源和 DAG WASM 制品；节点在受理时复制版本快照到本地执行目录，任务不写 NFS
- 新平台独立存储，不迁移旧 daemon/CLI 历史

## 相关

- [agents/control](../../agents/control/index.md)
- [agents/worker](../../agents/worker/index.md)
- [agents/node](../../agents/node/index.md)
- [项目管理](../../agents/project/index.md)
- [Agent Harness](../harness/index.md)
