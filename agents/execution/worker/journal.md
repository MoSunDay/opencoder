Commit: cea9499682d959503567277918b3f8da3d8115ba

# worker journal 子系统

执行记录的常驻摘要与按需读取；完整 `execution.json` 留在原位，磁盘格式不变。

## 模块
- [journal.rs](../../../crates/worker/src/journal.rs) — Journal 打开、恢复、保存与删除入口
- [journal/summary.rs](../../../crates/worker/src/journal/summary.rs) — 常驻调度元数据；冻结请求与配置留在磁盘
- [journal/loading.rs](../../../crates/worker/src/journal/loading.rs) — 阻塞线程逐条加载历史并提取摘要
- [journal/reading.rs](../../../crates/worker/src/journal/reading.rs) — 按需读取与修订核对
- [journal/cache.rs](../../../crates/worker/src/journal/cache.rs) — 完整正文与小详情的最近使用读取缓存
- [journal/inspection/mod.rs](../../../crates/worker/src/journal/inspection/mod.rs) — 不驻留冻结 runtime 的小详情视图
- [journal/io.rs](../../../crates/worker/src/journal/io.rs) — 缓冲写入、原子替换与目录同步
- [journal/projection.rs](../../../crates/worker/src/journal/projection.rs) — 上报投影
- [journal/reporting.rs](../../../crates/worker/src/journal/reporting.rs)、[reporting/](../../../crates/worker/src/journal/reporting/acknowledgements.rs) — 周期上报路径与确认
- [journal/compatibility.rs](../../../crates/worker/src/journal/compatibility.rs) — 布局兼容检查

## 验证
- [tests/journal_memory/](../../../crates/worker/tests/journal_memory/cases.rs) — 定向内存用例
- [scripts/acceptance/runtime_memory/probe.py](../../../scripts/acceptance/runtime_memory/probe.py) — Runtime 内存对比脚本

## 相关
- [worker 模块](index.md) — 所属模块索引
