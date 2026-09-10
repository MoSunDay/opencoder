Commit: c36ac68df313ec108549ed5b95756eb37edf5f69

# core 模块

跨 crate 共享类型与 Config 单一真源。

## 关键路径
- `src/message.rs` — Message/Role/ContentBlock；serde tag `kind`；`estimate_chars` 全块覆盖。
- `src/config.rs` — `Config::load`：候选深度合并（project 覆盖 global）→ 域文件 → env 变量。
- `src/config/env.rs` — 环境层 `~/.opencoder/envs/<active>/config.json` 插入候选链。
- `src/config/domain.rs` — mcp/cli/skills/ap 四域独立文件，项目层整体遮蔽。
- `src/config/keymap.rs` — `KEYMAP_INFO` 17 个可重绑定 TUI 动作。
- `src/config/cli.rs` — `InjectionTarget {parent,explore,build}` 注入目标。
- `src/config/autopilot.rs` — `ApMode off|ap|review` 三态。
- `src/net.rs` — `build_http_client`/`effective_proxy`，connect 30s；lib.rs re-export。
- `src/data_dir.rs` — `data_dir_for(workdir)` per-workdir 数据目录唯一解析。
- `src/harness/mod.rs` — `Harness::{Opencode,Codex}`、`pin_settings`。
- `src/harness/settings.rs` — `CodexSettings` 校验与独立 argv。
- `src/harness/runtime.rs` — `RunnerSettings` 带 revision 命名 Codex profile。
- `src/harness/scope.rs` — task-local Codex 设置/运行态，供重载 Config 的驱动读取。
- `src/agent/` — meta/resource/compose：引用卡 + 共享池；scope 任务局部资源根。
- `src/skill.rs` — 多根发现 first-wins 遮蔽；缓存 `src/skill/skill_cache.rs`。
- `src/tool.rs` — `Tool` trait / `ToolArc` / `ToolContext` / `ToolOutput`。
- `src/identity.rs` — `Identity`/`Role`(admin|root|user)/`token_hash`(sha256)；不从 lib 根 re-export（避让 message::Role）。
- `src/fleet/protocol.rs` — `PROTOCOL_VERSION = 8`，Server/Node 必须同代际。
- `src/fleet/{queue,scheduling}.rs` — FIFO/LIFO 排序与纯 CPU 节点选择。
- `src/share_fs.rs` — NFS 兼容共享树布局（todo/env/agent/tools）。
- `src/sse.rs` — `SseEvt` 服务端 SSE 事件类型。

Provider 配置在 `src/config/provider.rs` 声明 `chat_completions`（默认）与 `responses` 协议；`Message.provider_state` 保存 Responses 续聊所需的 provider、模型和有序 output，凭据不落库。

## 边界
- 域文件项目层存在即整体遮蔽外层，不逐键合并、不查 XDG。
- agent 解析优先当前执行固定的资源根（agent::scope），未设置走原解析。

## 相关
- [agents/session](../session/index.md) — Config 驱动压缩与模型选择。
- [agents/llm](../llm/index.md) — Message lowering。
- [agents/control](../control/index.md) / [agents/worker](../worker/index.md) — fleet 协议消费方。
