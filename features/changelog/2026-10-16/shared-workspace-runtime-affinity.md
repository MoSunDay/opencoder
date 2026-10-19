Commit: 00f0d5d3036841c6bac43069c4e08d8c8288fe38

# 跨发布子执行 Runtime 归属

Host 仅识别旧父字段时，当前共享工作区子执行会在发布后选择新 Runtime，无法读取原父目录。现在创建与能力探测共同识别当前共享目录、嵌套 Brain 及旧父字段，保留已经持久化的 owner 和无约束跨节点调度。Server 的探测仅转交必要归属字段及已有资源清单，不加入任务正文。

## 测试映射

- 父旧版本、活动新版本、探测无占位、创建与重放归属：`children_probe_create_and_replay_follow_the_parent_after_runtime_switch`。
- 共享目录与嵌套父缺失不回落新版本：`required_unknown_parent_never_falls_back_to_active_runtime`。
- Server 转交父字段并保留旧清单：`probes_keep_parent_affinity_and_legacy_manifests_without_task_content`。
- 定向范围：Agent Host 两项；Control capabilities 模块；相关 crate Clippy。真实容器跨发布和 900 秒观察结果记录在发布回执，未完成前不称验收通过。

参见 [Host](../../../agents/agent/index.md)、[运行协议](../../../docs/brain-orchestration.md)。
