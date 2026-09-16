Commit: d146e517f8b31ba3f8e5a1e493e0450d0c50d624

# agent 模块

稳定 Host 与独立版本 Runtime 的节点入口。

## 索引
- `crates/agent/src/` — Host/Runtime 装配与兼容入口
- `src/host/service.rs` `route()` Maintenance 分支 — 节点级维护命令中继：`dialogs_clear`
  活跃 runtime 转发 worker、休眠 runtime 从 `runtime_sleep` saved inventory 剔除可删行
  （sync_inventory 全量上报会复活 control 侧已删行，host 必须同步清）
