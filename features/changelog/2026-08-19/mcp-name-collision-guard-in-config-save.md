Commit: (working-tree, post-7a9f188)

# MCP server 名冲突守卫下沉到 core `Config::save`，web PATCH 命中返回 400

## 背景

bug #14 语义：两个 MCP server 名若经 `[-.]→_` 归一化后相同（`a-b` / `a.b` / `a_b`），注册时共用一个 `mcp__a_b__*` 工具前缀，后者静默遮蔽前者的工具并绕过其 `inject_to` 作用域。此前唯一的守卫在 TUI 表单层：`crates/tui/src/mcp_menu/patch.rs::colliding_server`（含 `renamed_from` 豁免与原地更新豁免），由 `app_loop_mcp.rs` 在 `/mcp` 表单保存前调用。core `Config::save` 落盘前只查 `model` 畸形守卫，**没有任何 mcp_servers 冲突检查**；web `PATCH /api/config` 任意 JSON patch 直通 `Config::save`，写入 `{"a-b":…, "a.b":…}` 会被静默接受，且出错一律 `error_500`。

## 根因

- 守卫只存在于 TUI 表单路径：远端 web PATCH（以及任何直接调 `Config::save` 的调用方）绕过它。
- `save` 的分流（split-routing）使 `mcp_servers` 永远写入 mcp.json 域文件（`domain::save_domain`），`save_to` 的 config.json 路径不经过它——守卫必须放在域文件写入点才有效。
- web 层把所有 save 错误映射为 500，客户端无法区分"补丁本身非法"与"服务端故障"。

## 变更

- **`crates/core/src/config/mcp_guard.rs`（新）**：纯函数守卫模块——
  - `normalized_server_name`：`[-.]→_` 一行函数，与 `crates/session/src/mcp/tool.rs::sanitize_server_name`（注册侧）、`crates/tui/src/mcp_menu/patch.rs::normalized_server_name`（表单预检侧）按现有惯例三处复制、注释互引、各自带表驱动 pinning 测试。
  - `mcp_name_collision(&Map)`：对**非 null** 键两两归一化碰撞检测，返回 `(offending, existing)`。在 merge 后的视图上检查：rename 的旧键已被同 patch 的 `null` 删除、原地更新只留一个键，故 TUI 预检的 `renamed_from`/原地更新豁免在 save 时机**天然等价成立**（doc 注释已说明）。
  - `conflict_message`：错误文案含两个原始名与归一化前缀（`…"a-b" collides with existing "a.b" (both normalize to mcp__a_b__…)`）。
  - `mcp_name_conflict_in_patch(workdir, patch)`：dry-run 预检（只读），委托 `domain::probe_mcp_conflict`。
- **`crates/core/src/config/domain.rs`**：
  - 抽出 `read_root(target)`（缺失/空白→`{}`，损坏→拒绝）供 `save_domain` 与 probe 共用，dry-run 看到的与真实 save 完全一致。
  - `save_domain` 在 merge + 归一化之后、**序列化/写盘之前**对 `key == "mcp_servers"` 执行守卫（mcp.json 顶层即 server map），命中 `anyhow::bail!`，文件零写入。
  - 新增 `probe_mcp_conflict`：复刻 `split_patch` 路由 + `write_target` + `read_root` + merge + 守卫，返回冲突文案；读失败返回 None（交给后续 save 报错）。
- **`crates/core/src/config.rs`**：`save_to` 在 merge 之后对 root 级 `mcp_servers` 同样执行守卫（防御 `save_global`/空 patch 回退等仍经 config.json 的路径；正常 save 已分流到 mcp.json）。`pub(crate) mod mcp_guard` + `pub use mcp_guard::{mcp_name_collision, mcp_name_conflict_in_patch}`。
- **`crates/web/src/api_ops.rs::patch_config`**：save 前调用 `mcp_name_conflict_in_patch` 预检，命中返回 `error_400`（本文件内新增同款 helper，与 api.rs/api_envs.rs 各自持有的惯例一致）；无冲突走原 `Config::save`。check-then-save 并发窗口可接受——core 落盘守卫兜底，竞态不可能污染配置文件。
- **TUI 不变**：`/mcp` 表单预检（`colliding_server`）与报错文案保持原样，仍是第一道网（表单级错误先于 core 守卫给出）；core 守卫是第二道网。TUI 的 rename/null patch 在 merge 语义下天然通过 core 守卫。

## 测试清单

- core 单测（`crates/core/src/config/tests.rs`）：
  - `save_rejects_mcp_servers_normalized_collision` — 红→绿：修复前 `Config::save` 接受 `{"a-b":…, "a.b":…}`（写入 `.opencoder/mcp.json`），修复后 Err（文案含两名 + `mcp__a_b__`）且所有 mcp.json 候选位置零污染。
  - `save_allows_rename_via_null_delete_marker` / `save_allows_intra_patch_rename_on_fresh_file` — rename（`a-b`→`a.b`，同 patch 带 `a: null`）与全新文件上的 intra-patch rename 均 Ok。
  - `save_without_mcp_servers_key_is_unaffected` — 无 mcp_servers 的 patch 不受影响。
- core 单测（`crates/core/src/config/mcp_guard.rs` 内嵌）：`normalized_server_name_is_table_driven`（pinning）、`collision_detects_normalized_twins`、`collision_ignores_null_delete_markers`、`collision_ignores_disjoint_and_single_entries`、`collision_catches_three_way_normalized_clash`、`conflict_message_names_both_and_normalized_prefix`。
- web 集成测试（`crates/web/tests/web_api_ops.rs`）：`patch_config_mcp_name_collision_returns_400` — 红→绿：修复前 PATCH 碰撞返回 500（断言 400 失败），修复后 400 + body 含两名与 `mcp__a_b__`、mcp.json 候选零污染，随后合法单 server PATCH → 200 且落盘。
- 回归：`cargo test -p opencoder-core -p opencoder-web` 全量 34 个测试二进制 0 失败；`cargo test -p opencoder-tui --lib -- mcp` 40 通过（在 HEAD+本次改动的隔离 worktree 上验证，排除同仓并行任务的在途改动干扰），含 `handle_mcp_outcome_refuses_save_colliding_after_normalization`（TUI 预检不回归）；`cargo clippy -p opencoder-core -p opencoder-web --all-targets` 0 warning。

## Impact Surface

- 所有 `Config::save` 调用方（TUI 各菜单 / web PATCH / cli）：写入会在归一化后撞车的 `mcp_servers` 名时收到 `CoreError::Config`（域文件路径文案前缀 `save domain file for \`mcp_servers\``）。正常保存、rename（null 删除标记）、原地更新、toggle、delete 均不受影响。
- web `PATCH /api/config`：mcp 名冲突从 500 变为 400（body `{"ok":false,"error":…}` 含冲突详情）；其余错误语义不变。
- 边界：历史上已被污染（现存两个撞车名）的 mcp.json，任何后续经 `Config::save` 的 mcp_servers 写入（含纯 toggle）都会被拒绝，直至手工清理——这是 save 时机全量检查的固有语义。
- 行数：config.rs 798 / domain.rs 588 / tests.rs 526 / api_ops.rs 268 / web_api_ops.rs 646（迭代 ≤800），新文件 mcp_guard.rs 161（≤400）。
