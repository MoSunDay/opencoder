Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# dag-binary 模块

Linux 原生二进制的不可变版本池。负责名称、资源引用、ELF 与版本文件校验，不负责启动进程。

## 主要接缝

- [validate.rs](../../crates/dag-binary/src/validate.rs)、[token.rs](../../crates/dag-binary/src/token.rs)：纯校验 ELF64、小端编码、架构及 `name@vN` 引用；执行节点再核验自身架构。
- [meta.rs](../../crates/dag-binary/src/meta.rs)：池指针、版本清单、文件大小与 SHA-256。
- [write.rs](../../crates/dag-binary/src/write.rs)、[lock.rs](../../crates/dag-binary/src/lock.rs)：互斥发布、原子落盘与回滚；中断发布也不复用版本号。
- [共享 HTTP 处理器](../../crates/web/src/api_dag_binaries.rs)：发布、查询、下载、回滚与删除；控制面和独立资源服务操作同一个配置池。
- [运行资源固定](../../crates/dag-runtime/src/resources.rs)：受理时固定版本及摘要，恢复读取本次保存的程序，不重新解析 current 指针。

## 边界

二进制和源工作区分别由只读 NFS 导出；写入管理接口需要认证，不通过 NFS 修改资源。

## 相关

- [执行约定](../../rules/04-dag-execution-contract.md)、[DAG 能力](../../features/dag/index.md)
- [control](../control/index.md)、[dag-runtime](../dag-runtime/index.md)、[worker](../worker/index.md)
