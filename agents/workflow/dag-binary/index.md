Commit: 573bb2e48fa20832a47b7bec8adc8ee3c08e8e78

# dag-binary 模块

Linux 原生二进制的不可变版本池。负责名称、资源引用、ELF 与版本文件
校验，不负责启动进程。

边界：二进制和源工作区分别由只读 NFS 导出；写入管理接口需要认证，
不通过 NFS 修改资源。

## 索引
- [validate.rs](../../../crates/dag-binary/src/validate.rs) — 名称与 ELF 纯校验
- [token.rs](../../../crates/dag-binary/src/token.rs) — `parse_resource_token` 解析 `name@vN` 引用
- [meta.rs](../../../crates/dag-binary/src/meta.rs) — 池元数据、根解析与纯路径助手（读侧，静默降级）
- [scope.rs](../../../crates/dag-binary/src/scope.rs) — 任务作用域池根，并发执行各用各的快照
- [write.rs](../../../crates/dag-binary/src/write.rs) — 保存/回滚/删除；原子写且版本号不复用
- [lock.rs](../../../crates/dag-binary/src/lock.rs) — 发布互斥文件锁
- [api_dag_binaries.rs](../../../crates/web/src/api_dag_binaries.rs) — 共享 HTTP 处理器：控制面与资源服务操作同一配置池
- 资源装配细节（`frozen_resources`）见
  [dag-runtime 模块索引](../dag-runtime/index.md)

## 相关
- [执行约定](../../../rules/04-dag-execution-contract.md)、[DAG 能力](../../../features/platform/dag/index.md)
- [control](../../platform/control/index.md)、[dag-runtime](../dag-runtime/index.md)、[worker](../../execution/worker/index.md)
