Commit: 2686d40a267436adb555041fc8154fc9bb454574

# Brain 流程与 DAG 日志修复

## Context

流程动作在暂停后取消时可能被错误折叠为失败；DAG agent 使用仅完成帧的模型响应时，结构化结果和实时步骤日志也可能缺失。

## Change Summary

- 取消中的 Brain run 在没有活动子动作后收敛为 `cancelled`，不会继续执行流程转移。
- agent 步骤从已持久化的最新 assistant 消息恢复非流式完成文本，并写入 `step_log`。
- 执行日志面板支持步骤筛选、历史分页、实时回退、游标去重和连接失败提示。

## Impact Surface

Brain execution state machine、DAG runtime event stream、SPA execution log view。

## Related Docs

- [Brain logic](../../../agents/brain/index.md)
- [DAG runtime logic](../../../agents/dag-runtime/index.md)
