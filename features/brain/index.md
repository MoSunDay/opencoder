Commit: 3b4775905c950f64433b5c9f4439f4396674b3c6

# 大脑调度工作台

计划仅保存 step 和连线；每个 step 是一句话任务及一个能力引用，也可引用固定版本的可执行计划。计划库支持编辑、浏览器草稿、版本保存和启动。

画布自上而下、同层并行；全层成功才激活下一轮，节点尝试耗尽则失败并取消同层未结束执行。单节点最多尝试 1–5 次（默认 2 次），计划最多 256 个节点、每层 32 个节点，子计划嵌套深度上限为 3。运行页显示层屏障、最新尝试和决策索引。点击节点或历史尝试复用能力的运行组件查看明细，子计划也使用同一组件。

仅支持 schema_version 4，无旧版本工作台。运行读取 `/api/brain/runs/:id/layered`，层详情读取 `/layered/rounds/:layer`，事件与定时轮询刷新。暂停、恢复和取消使用运行命令。

- [运行协议](../../docs/brain-orchestration.md)
- [brain 模块](../../agents/brain/index.md)
- [control 模块](../../agents/control/index.md)
- [web 模块](../../agents/web/index.md)
