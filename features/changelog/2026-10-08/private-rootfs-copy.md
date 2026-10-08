# 私有容器镜像复制

真实容器会话在等待 idle 的 180 秒内仍停留在镜像复制：共享镜像约 240 MB，Git 的 133 个硬链接别名被逐个复制后扩张到约 681 MB。每 1 MiB 同步一次又重复等待刷盘。

保留同一个私有镜像内部的硬链接，首份文件仍独立复制，绝不连接到共享源镜像或其他运行。数据按 8 MiB 批次同步，文件完成时仍同步权限和剩余数据。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 私有字节、内部别名、权限、超过同步批次的尾部及冻结重试 | large_image_files_keep_independent_bytes_permissions_and_frozen_retry | crates/dag-runtime/src/sandbox/rootfs.rs |
| 容器会话真实运行和第二轮继续 | sandbox_agent_session_runs_and_continues_in_runc | tests/operator_e2e/agent_sandbox.rs |

验证尚在执行；历史全量测试超时记录保留，不能计为通过。
