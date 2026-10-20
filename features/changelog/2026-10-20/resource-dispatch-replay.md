Commit: 309c02570d3cbef348279d7db7839b45e3538294

# 重试已保存的派发请求

同一执行等待外部资源后，Server 会重复准备已经保存的请求。原分支在事务内直接返回成功，连接池检测到事务仍未结束后回滚并报错，导致原执行无法继续申请资源。

相同请求现在先结束读取并提交事务再返回，保留冻结输入、节点归属、定义、派发回执和执行状态。请求冲突仍失败，连接池仍拒绝未完成的成功事务。资源派发重试失败记录执行 ID、HTTP 状态和固定错误分类，不记录请求正文或凭据。公开接口、数据库结构和重试间隔保持原约定。

19 项存储、数据库和资源派发定向测试通过；受影响 crate 的 Clippy 零警告。发布脚本测试运行 152 项，其中 1 项按环境条件跳过。

## 验证映射

| 行为 | 定向验证 |
| --- | --- |
| 重启后重复准备，保留冻结定义及已确认状态；冲突后可继续重试 | `repeated_prepare_closes_transactions_and_preserves_frozen_dispatch` |
| 未完成事务仍回滚并拒绝，后续操作可继续 | `unfinished_transaction_is_rolled_back_and_rejected`、`dropped_native_transaction_preserves_the_original_error_and_rolls_back` |
| 派发归属、迟到确认及项目重试约定 | `store/tests/handoff.rs`、`store/tests/project_dispatch.rs` |
| 资源不可用时持久等待，重启保留提供方及等待请求 | `control/tests/resource_admission.rs` |
| 恢复授权后按顺序执行且不重复创建，取消不创建远端执行 | `control/tests/e2e/resource_admission.rs` |
| 原生容器与任务跨版本继续执行、公共入口持续受理 | `scripts/acceptance/smooth_release/main.py`、`live.py`，发布证据保存在仓库外 |

相关职责见 [Store](../../../agents/store/index.md) 和 [Control](../../../agents/control/index.md)。
