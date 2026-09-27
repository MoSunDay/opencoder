Commit: 6b5b4d28e047faa5b8d8468d6733c68cfd1e8685

# 平台能力盘点实锤验证（proof）

对同日盘点候选问题 P1–P10 逐项实锤：单执行者只读复核，未修改仓库；逐项结论与证据（前置条件、复现、影响、调用链）在产物 04：
`/data00/workspace/artifacts/opencoder-platform-audit/brain-opencoder-platform-audit-20260927/`。

须修复（有当前实际影响；均已在同日工作区由根执行者修复，本节点只读核对）：

- P1 文档子命令漂移：README.md / README.en.md / docs/quickstart.md / docs/quickstart.en.md 已改 `opencoder-server`/`opencoder-agent`，复扫旧命令 0 命中；旧脚本 `daemon --server` 静默不起服务的行为随迁移提示保留。
- P2 README 架构描述过时：crate 计数与结构图已改为现行分层（members 实数 23）。
- P10 daemon 显式 token 回显：`migration_hint` 不再拼接 token、改为提示 `--token-file`；单测改负向断言（定向测试 8 passed / 0 failed，根执行者回执）。复现仅用无效哨兵值，严禁真实凭据。

排除（勿再重复排查）：P4 ui_device_v1 平台"能力要求"与设备服务"未广告"是协商的两层语义，缺能力提交被 409 拒绝、行为自洽（见 changelog 2026-09-24）；P6 node=运行时循环、worker=执行适配器，`crates/agent/src/main.rs` 组合点成立；P7 SPA dist 漂移是并发开发中间态，发布链有 `check-spa-drift.sh` 强制门禁；P9 清理白名单有意排除能力库表。

维护性观察（当前无故障，未列入修复范围）：P3 `crates/web::serve` 死入口、双路由集并存；P5 `device_count` 硬编码 18 与设备池容量无共享常量（当前 18=18 一致）；P8 agents/* pin 手工滞后。证据不足：无。

## 相关

- [盘点（01–03 产物）](platform-capability-audit.md)
- [能力地图](../../../features/index.md)
- [local 模块](../../../agents/local/index.md)
