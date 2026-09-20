# 容器镜像复制期间保持接收连续性

生产信号回滚与再次发布两次出现接收延迟超出 1 秒，分别为 1.554 秒和 1.110 秒。两次慢请求都属于旧 Runtime，且与新 Runtime 的 OCI 探针复制镜像重合。完整镜像约 483 MiB，其中 Codex 可执行文件约 294 MiB；整文件复制积累大量脏页，会拖慢同一文件系统中其他请求的持久化同步。

保留每个容器独立的真实 rootfs、不可变重试、权限和符号链接处理，将普通文件复制改为每次最多 1 MiB 并同步数据，文件完成后同步权限。复制或同步失败继续向上传递，未完成的暂存目录不能发布。没有共享可写文件或修改任何验收时延门槛。

在生产同一磁盘上的独立对照实验中，复制同一个约 294 MiB 文件，整文件复制时旁路同步最高等待 325/170 毫秒；分块同步对应 10/15 毫秒。实验只支持写入突发的因果分析，不替代完整发布验收。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 大文件逐字节完整、权限与独立 inode、已有快照重试不变化 | `large_image_files_keep_independent_bytes_permissions_and_frozen_retry` | `crates/dag-runtime/src/sandbox/rootfs.rs` |
| 镜像和设备目录相互隔离、符号链接与历史快照 | `snapshots_isolate_images_devices_and_retries` | 同上 |
| 非法挂载父目录不发布、不遗留暂存目录 | `invalid_mount_parent_does_not_publish_or_leave_staging` | 同上 |
| 真实 OCI 存活、三版本交接、回滚、接收和调度均低于 1 秒 | `smooth_release/main.py` 与 `smooth_release/live.py` | `scripts/acceptance/smooth_release/` |

原始对照实验：`/var/tmp/opencoder-latest-release-20260920/oci-copy-io-diagnostic/result.json`。两轮失败生产证据分别在同目录下的 `release-live-c59411ac6a608a74`、`release-live-9e3fa065e54ac22d`，均保留。全量回归、发布包和最终上线结果另记发布回执。
