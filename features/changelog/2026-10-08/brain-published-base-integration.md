# 大脑调度纳入当前已上线版本

纳入 `c9cf67d2` 已上线的原生结果读取、代理排除与普通执行容量恢复，保留大脑完整上下文预算、意外崩溃失败和终态控制接口规则。上下文预算仅保留目录模块实现，删除重复文件。

普通执行容量恢复仍需完整执行记录、准确 Runtime 身份和内核进程清理证据。Brain 的未结清运行票据拒绝自动恢复，继续要求 `storage settle-brain-crash` 对精确根运行进行离线结算。

验证映射：`failed_brain_requires_explicit_settlement_and_keeps_its_capacity` 检查失败根票据保留、运行记录不变且其他执行不能占用该容量；`brain_crash`、`brain_context_budget`、`brain_scheduler_v4` 和原生结果读取相关测试验证合并影响。按用户要求仅验证相关模块及其真实运行链路。
