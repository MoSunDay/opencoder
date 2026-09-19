Commit: c869027bad052c7ce91bdd0818f7be149b2b66dc

# agent 模块

稳定 Host 与独立版本 Runtime 的节点入口。

## 索引
- `crates/agent/src/` — Host/Runtime 装配与兼容入口
- `src/host/service.rs` `route()` Maintenance 分支 — 节点级维护命令中继：`dialogs_clear`
  活跃 runtime 转发 worker、休眠 runtime 从 `runtime_sleep` saved inventory 剔除可删行
  （sync_inventory 全量上报会复活 control 侧已删行，host 必须同步清）
- `src/host/mod.rs` run() — fleet channel 按 `host.db fleet_definitions` 中
  **每个 enabled 的 `release_server` 定义**各起一路 `opencoder_node::fleet::run`
  循环（无定义时回退 `--remote`）；定义含 `url`（须 loopback HTTP）。管理口为
  host 本地 API `POST :port/servers/<id>`（Bearer，body `{url,enabled}`）。
  陷阱：下线旧 release 后若遗留 enabled 定义指向死端口，host 会每 2s 打
  `channel disconnected Connection refused`，且多路循环对活跃 server 产生
  `node already` 重复注册竞态（案例与处置见
  `../features/changelog/2026-09-18/release-0662b924-signal-deploy.md`）。
