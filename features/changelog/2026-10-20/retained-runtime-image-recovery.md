Commit: f90df21319a94ee4e7d2b560c32a25a61dff68ad

# 保留 Runtime 升级后的 DAG 恢复

存储引擎维护为保留 Runtime 更换程序及私有容器镜像后，原 DAG 仍保存旧镜像路径。恢复检查因此会拒绝节点与运行器版本不匹配的任务。

维护制备现在保存限定到原 Runtime 的镜像替换回执，记录允许继续的历史镜像和新运行器摘要。恢复只有核对该回执后才使用新私有镜像，完整构建信息检查继续生效。原配置、输入、资源和执行路径保持不变；普通配置漂移、其他 Runtime 的回执、运行器损坏或回执替换均不能获得恢复许可。

对已完成引擎维护的保留 Runtime，内部修复工具可在正常发布完成后单独替换休眠程序。必须满足已退役、Host 已保存休眠记录、没有预留或进程；更新期间持有 Host 与节点锁。工具保留原 unit 和程序记录，重载失败时恢复，重试复用同一份回执。它不会重启 Server、其他 Runtime 或自动恢复任务。

维护入口为 `python3 scripts/platform/rolling/maintenance/engine/repair.py --config <deployment-config> --runtime <retained-runtime-id> --release <current-release-id>`。运行后通过原执行恢复入口验证；工具安装成功不表示原执行已经完成。

验证映射：

- 制备与连续升级 → `RetainedProgramTests.test_separate_program_preserves_identity_config_resources_and_old_image`、`RetainedImageReceiptTests`。
- 无回执、错误归属、摘要变化及链接拒绝 → `operations::dag_preflight::image::tests`。
- 原 DAG 真实容器恢复、已完成步骤只执行一次、冻结输入与配置保留 → `retained_dag_uses_explicit_image_upgrade_without_replaying_completed_steps`。
- 休眠程序安装、重复调用、活跃预留、Host 唤醒锁、归属变化、失败回退与原回执重试 → `SleepingRuntimeRepairTests`。

已通过 4 项 Worker 单元测试、1 项真实 `runc` 恢复测试、6 项原镜像制备与回执测试、6 项休眠修复测试，以及 `opencoder-worker --all-targets` Clippy。发布和真实运行验收结果由发布回执记录；上述测试不代表已上线。
