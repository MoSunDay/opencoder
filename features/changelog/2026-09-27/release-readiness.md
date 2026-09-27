Commit: 1565c74a3f348aeb0d550a2aae6bdcdabaf52ff5

# 发布候选就绪判断

Host 保留退休 Runtime 的休眠库存以继续提供历史执行索引，但新任务准入只取活动 Runtime 的健康状态。发布作业在旧 Host 的历史资源错误导致候选 Server 暂时看不到就绪节点时，核验候选 Host、候选 Runtime 和候选 Server 身份，再进入切换；切换后仍通过公共入口执行真实探针。

[Agent 模块](../../../agents/agent/index.md) · [Agent 调度平台](../../agent-platform/index.md)
