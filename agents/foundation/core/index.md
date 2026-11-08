Commit: 573bb2e48fa20832a47b7bec8adc8ee3c08e8e78

# core 模块

跨 crate 共享类型与 Config 单一真源。细节以代码为准。
接缝：`Arc<dyn Store>`、`Arc<dyn ChatStream>` 在相邻 crate 定义；Config 供全仓
加载。

## 索引
- [build.rs](../../../crates/core/build.rs) — 构建期提交与干净状态捕获
- [version.rs](../../../crates/core/src/version.rs) — 构建期版本元数据
- [src/message.rs](../../../crates/core/src/message.rs) —
  `Message`/`Role`/`ContentBlock`
- [src/tool.rs](../../../crates/core/src/tool.rs) — `Tool` trait、`ToolContext`、
  `ToolOutput`
- [src/config.rs](../../../crates/core/src/config.rs)、
  [src/config/](../../../crates/core/src/config) — Config 加载与合并
  （`load_with_home`、`load_with_home_frozen`、`load_operator`、
  `effective_domain_value`、`domain_file_for`）
- [src/config/runtime/](../../../crates/core/src/config/runtime) — Agent、DAG、
  Ontology、MCP、CLI、技能等运行配置
- [tests/config_overlay/](../../../crates/core/tests/config_overlay/main.rs) —
  配置覆盖契约测试
- [identity/permissions.rs](../../../crates/core/src/identity/permissions.rs) —
  共享 HTTP 授权纯函数
- [src/harness/](../../../crates/core/src/harness) — 执行器选择与私有运行态
  （`Harness`、`fresh_runtime`、`matches_requested_env`）
- [harness/settings.rs](../../../crates/core/src/harness/settings.rs) — 类型化
  Codex 设置
- [harness/remote.rs](../../../crates/core/src/harness/remote.rs) —
  `ServerConnection`、`ServerCapability`
- [src/agent/](../../../crates/core/src/agent)、
  [src/skill.rs](../../../crates/core/src/skill.rs) — Agent 注册表与引用卡、
  技能发现；memory 池聚合在
  [agent/memory.rs](../../../crates/core/src/agent/memory.rs)
- [src/skill/seed.rs](../../../crates/core/src/skill/seed.rs) — 内置技能增量 seed
- [tests/skill_contract/](../../../crates/core/tests/skill_contract/main.rs) —
  技能契约测试
- [platform/](../../../crates/core/src/platform/mod.rs) — 本地会话与执行节点
  共用的宿主操作
- [net.rs](../../../crates/core/src/net.rs) — 代理感知 HTTP 客户端构造
- [src/data_dir.rs](../../../crates/core/src/data_dir.rs) — 按工作目录的
  数据目录解析
- [fleet/protocol.rs](../../../crates/core/src/fleet/protocol.rs) — Server/Node
  协议（`PROTOCOL_VERSION`）
- [fleet/release.rs](../../../crates/core/src/fleet/release.rs) — 发布交接
- [fleet/report.rs](../../../crates/core/src/fleet/report.rs) — 节点报告
- [fleet/inventory/](../../../crates/core/src/fleet/inventory/mod.rs) — 有界变更
  历史与分页快照
- [fleet/private_files/](../../../crates/core/src/fleet/private_files) —
  `PrivateExecutionContext` 与不可变私有任务文件
- [src/brain/](../../../crates/core/src/brain) — 版本化计划与编排线协议，
  `layered/` 为分层计划；IO 协议在
  [brain/io.rs](../../../crates/core/src/brain/io.rs)
- [ontology/](../../../crates/core/src/ontology/mod.rs) — 公开能力描述与平台入口
  清单
