Commit: 7687b5f581254ee6d826d8644789e7d498e761ba

# server 模块

版本 Server 与独立只读资源服务入口。

## 索引
- `crates/server/src/` — 二进制装配；复用 control/web 能力
- [control/release/resources.rs](../../crates/control/src/release/resources.rs) — 独立资源服务持有 Agent、Linux 二进制与源工作区的只读 NFS 导出；经认证的二进制管理接口与版本 Server 使用同一实现和配置池。
- 资源服务的 `GET /api/health` 返回 `role=resources` 与完整构建信息；兼容发布在预热前核对生产资源的版本。旧服务缺少元数据时要求维护升级。
- [rolling/maintenance](../../scripts/platform/rolling/maintenance/__init__.py) — 不兼容升级的持久阶段：冻结接入、排空、停服、备份、升级资源与挂载、启动迁移、私有验证、复开；迁移启动意图先落盘，此后只允许同一候选续跑或新格式版本修复。

常规兼容发布保留资源服务与已有挂载；数据契约升级通过维护流程切换。参见 [发布说明](../../docs/smooth-release.md)、[DAG 执行约定](../../rules/04-dag-execution-contract.md)。
