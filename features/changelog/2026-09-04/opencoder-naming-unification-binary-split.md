# opencode→opencoder 全仓命名统一 + 三二进制拆分验证 + 主二进制去 nfsserve

## Context

用户指出产品名是 **opencoder**，但仓库残留大量少-r 的 `opencode` 拼写：fleet worker 的 bin 名就叫
`opencode-agent`（包名却是 `opencoder-agent`）、自定义 Agent crate 包名 `opencode-agents`、core 公开函数
`global_opencode_home`、tmux 会话前缀 `opencode-`、以及遍布 doc/help/e2e 的 `opencode ts` / `opencode daemon`
文案。同时需按既定 P0 拆分验收：opencoder（tui/cli）与 opencoder-server（api/dag/brain/web 前端）确实分离、
前端只走 server、并控制 tui/cli 二进制体积。

## Change Summary

- **包/bin 统一**：`opencode-agents` crate → `opencoder-agents`；bin `opencode-agent` → `opencoder-agent`；
  workspace/session/web 依赖别名同步；代码引用 `opencode_agents::` → `opencoder_agents::`。删除根 Cargo.toml
  里指向不存在 `crates/client` 的陈旧 `opencoder-client` 条目。
- **标识符/文案统一**：`global_opencode_home` → `global_opencoder_home`（含 `<global_opencode_home>` 文档占位符）；
  `RUNTIME_NAME`/`opencode-nfs`/`opencode.tmp`/`~/.opencode`（write.rs 真笔误）等运行时字符串与全部 doc/help/
  测试 argv0 修为 opencoder 拼写；`opencode daemon` 迁移提示指向 `opencoder-server`/`opencoder-agent`。
- **tmux 前缀**：`TMUX_PREFIX` → `"opencoder-"`；`id_from_name` 双前缀识别（旧 `opencode-` 会话仍可 resume/cleanup，
  仅不再新建）；naming/tmux/actions 测试夹具更新并显式保留 legacy 识别用例。
- **主二进制去 nfsserve**：`tools_paths`（读路径）从 agents crate 挪入 `opencoder_core::agent::resource`（原实现本就是
  core 三函数的纯委托），session/agent_pools 改调 core；session 不再依赖 `opencoder-agents`，nfsserve+写路径只随
  `opencoder-server`（web）链接。`cargo tree -p opencoder` 已无 nfsserve/agents crate。
- **e2e 修正**：`scripts/e2e/web_scenarios.py` 原来启动早已不存在的 `opencode serve` 子命令 → 改为解析同 target 目录的
  `opencoder-server` 二进制直连；`tests/support` 的 fleet 二进制候选表收敛为单一正式名并更新说明。
- **不改动**：README 与 sst/opencode 的竞品对比、`opencode.db` 竞品命名、notepad 忽略列表 `.opencode`（第三方目录）、
  features/changelog/** 历史记录、三处防回归测试（断言裸 `opencode` 文案不得复现）。

## 验收（拆分与体积）

| 项 | 结果 |
|----|------|
| `opencoder`（tui/cli）依赖闭包 | core/cli/tui/session/llm/store/shellguard/todos + dag 纯域（经 store），**无** web/brain/node/dag-runtime/project/team/agents/nfsserve |
| `opencoder-server` 闭包 | web(SPA include_bytes!)+brain+dag+project+team+agents(写路径+nfsserve) |
| `opencoder-agent` 闭包 | node+dag-runtime(VM/runc)，无 web |
| release 体积（strip+thin-LTO） | opencoder **17.0 MB**（去 nfsserve 后 -0.8）/ server 17.7 MB / agent 26.5 MB |

## 测试覆盖（规则 01）

| 功能 | 测试 | 结果 |
|------|------|------|
| core `tools_paths` 三 scope+current 跟随 | `agent::resource::tests::tools_paths_covers_all_three_scopes` | 通过 |
| tmux 前缀新建/legacy 识别/三形态 resume target | `ts::naming` 4 用例 + `ts` 模块 90 用例 | 通过 |
| agents crate 删模块后自洽 | `cargo test -p opencoder-agents` | 26 passed / 1 ignored |
| session agent_pools 改走 core | `cargo test -p opencoder-session --lib agent_pools` | 3 passed |
| core agent 域 | `cargo test -p opencoder-core --lib agent` | 45 passed |

## 回归

`cargo check --workspace --all-targets` 零错误；`cargo test --workspace` 全绿（见当轮输出）；工作树另有上一轮
未提交的 brain/project 改动（与本轮无关，未触碰）。
