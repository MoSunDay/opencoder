Commit: 6b5b4d28e047faa5b8d8468d6733c68cfd1e8685

# OpenCoder 平台能力盘点（只读）

对 workspace 做平台能力源码盘点：23 个 crate、Server/Node 版本面（`PROTOCOL_VERSION=10`、`opencoder-server --build-info`）、CLI/TUI/Web 入口、Brain/DAG/Team/TODO/Agent、设备调度与测试入口；区分平台实现与外部业务能力（deploy/device-manager、pc-issue、glm5.2/qwen-vision 验证脚本）。盘点以只读取证为主，同时订正了本地记忆索引；盘点窗口内 HEAD 从 `b8d46650` 前进到 `6b5b4d28`。产物（01 能力清单、02 候选问题、03 证据）在
`/data00/workspace/artifacts/opencoder-platform-audit/brain-opencoder-platform-audit-20260927/`。

确认的当前态事实（可直接核查）：

- README 宣传的 `opencoder server|client|serve` 子命令不存在（README.md:133-134 vs `crates/local/src/lib.rs` Command 枚举；`daemon --server` 仅打印迁移提示，`crates/local/src/daemon.rs`）。
- README.md:144 "8 个 crate" 与 members 实数 23 不符；README.md:303-306 列出不存在的 `crates/{client,cli}`。
- `crates/web/src/lib.rs` `serve` 无 crate 外调用方；现役路由在 `crates/control/src/routes.rs`，双路由集并存。
- 工作区 SPA 产物漂移：`scripts/check-spa-drift.sh` 实测 DRIFT（`spa/src/chat.jsx` 未提交改动晚于 dist 构建）。
- 记忆订正：agents.md `opencode-cli` 笔误改为 `opencoder-cli`（crates/ctl/Cargo.toml:2,8）；补齐 agents/dag-review-tools 模块索引。

转实锤中的安全疑点（daemon 显式 token 回显，候选问题 P10）：

- `crates/local/src/daemon.rs` `migration_hint`（server 分支 :56-58、client 分支 :69-71）将显式传入的 token 原样拼入迁移提示（`--token {t}`），单测 `server_hint_echoes_token_and_web_off` / `client_hint_carries_remote_name_token` 固化该行为；哨兵值实验（root-executor read-back）确认 stdout 回显。完整定位与证据见产物 02/03。
- 后续修复与验证见 [修复回执](platform-capability-audit-repair.md) 和 [验证回执](platform-capability-audit-verify.md)。

其余候选问题（ui_device_v1 跨仓库准入表述、`device_count` 上限硬编码、node/worker 命名边界等）未实锤，仅记录于产物 02。验证方式：只读 grep/文件核对 + check-spa-drift.sh 实测，证据见产物 03。

## 相关

- [实锤验证（proof）](platform-capability-audit-proof.md)
- [能力地图](../../../features/index.md)
- [逻辑地图](../../../agents.md)
- [dag-review-tools](../../../agents/dag-review-tools/index.md)
