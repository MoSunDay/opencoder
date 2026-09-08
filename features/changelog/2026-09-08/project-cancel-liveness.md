Commit: ff43bfa9410769695481374ea3bd2c5d7e08867c

# 项目取消保留运行标记与部分输出

项目取消后仍需刷新消息、模型响应和回放归档。过早移除运行标记会让重复取消误判驱动已经丢失，并用恢复提示覆盖实际输出；工具步骤已产生的 assistant 文本也可能把取消误判为成功。

`ProjectService::cancel` 保留令牌直到驱动完成收尾，重复请求只触发同一取消信号。Plan/Agent Act 共用取消优先的结果判定，保存部分输出并保持 cancelled 状态。

回归测试 `repeated_cancel_does_not_converge_a_driver_still_flushing_output` 覆盖重复取消、stale 扫描及最终输出保留；项目模块 28 项单元测试、14 项集成测试通过，真实二进制验收覆盖工具输出后的取消。

逻辑见 [project 模块](../../../agents/project/index.md)，业务规则见 [Agent 调度平台](../../agent-platform/index.md)。
