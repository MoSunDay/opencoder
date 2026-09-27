Commit: 24a1081aff7fd8591f860723bae5eb787edf68e9

# Brain 事件唤醒与人工引导

## 背景

schema 7 Brain 已按整层终态屏障推进，但运行中缺少人工输入触发的即时再决策；生产激活也需要统一到 OpenCoder 的 agent loop。

## 变化

- 运行详情和托管能力详情接受人工文本，写入根运行的 `human_input` 事件；新输入取消尚未完成的旧决策，阻塞态可据新信息重新决策。
- 层屏障未满足时只允许 `guide`。大脑可通过可重放、按序确认的事件引导当前运行中的 Agent/Operator；Team 在下一次成员发问时应用持久化引导。更新的人工输入会使尚未投递的旧引导失效。
- 生产模型激活使用 OpenCoder session agent loop；每次事件只处理一份上下文并输出一个决策。DAG 运行图仍需停止原运行后重新提交。

## 影响范围

Brain 决策协议与投影、Worker outbox 和 Team 执行、Control 输入与引导投递、运行详情界面及 CLI 模型激活。

## 约束

Team 正在生成中的单个成员回答不会被打断；引导从下一次成员发问起生效。人工输入不越过层终态屏障，也不直接改写已提交的 DAG。

## 相关文档

- [大脑工作台](../../brain/index.md)
- [Brain 逻辑](../../../agents/brain/index.md)
- [运行协议](../../../docs/brain-orchestration.md)
