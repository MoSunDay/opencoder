Commit: 37ee51ce6274e0ade8eb9fc8a34da1a1b7e7aa57

# 执行 IO 升级与回滚兼容

根请求受理时固定 IO 版本，子执行与嵌套 Brain 继承。缺少标记的历史任务不增加 context 字段、产物绑定或节点能力要求，新 Worker 同样按旧协议生成决策和适配结果。

派发重试先重放原 Assignment，保持完整输入、执行身份、节点与指纹。根请求只有 claim 时，按原指纹确认具体协议，不能用当前默认版本覆盖它。

新绑定启用分 reader 过渡版和 writer 版；二进制与发布包携带相同的读取/生成版本。格式 10 标记 writer 的新持久化绑定，旧脚本也会拒绝危险回滚；reader 过渡版保留格式 9，由当前工具校验后允许与格式 10 重叠。发布前检查回滚基线，回滚前检查保留 Runtime 可能生成的协议。不改变数据库 schema，也不删除或改写认证数据。

功能 → 验证：

- 旧 context 与提示、产物绑定校验 → `legacy_context_keeps_the_old_wire_and_cannot_publish_artifact_bindings`
- 原请求重放与原节点 → `replay_legacy_children_preserves_fingerprint_inputs_and_owner`
- 只保存 claim 的根请求升级/回滚 → `claim_only_roots_keep_the_original_wire_during_upgrade_and_rollback`
- 嵌套协议继承 → `nested_admission_inherits_the_parent_contract_instead_of_the_new_default`
- 历史节点要求 → `legacy_root_and_children_do_not_gain_new_io_requirements`
- 新 Worker 服务旧 context → `legacy_root_rejects_context_upgrade_and_worker_first_cannot_publish_new_bindings`
- 发布与回滚保护 → `ExecutionIoTests`
- 实际旧版、过渡版、新版进程及下载 → [execution_io_upgrade/main.py](../../../scripts/acceptance/execution_io_upgrade/main.py)

相关约定：[Brain](../../../rules/06-brain-scheduling-contract.md)、[发布](../../../docs/smooth-release.md)。
