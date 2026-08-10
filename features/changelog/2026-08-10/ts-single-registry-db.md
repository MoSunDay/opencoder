Commit: 0e5fd33f9bccae7cc006da8429fe83142e1f2931

# 提升 `opencoder ts -l` 速度：ts 内容集中到一个 db

## Context

`ts -l`（及 `-r/-d/-c`）每次都要扫描 `<data_root>/<hash>/opencoder.db` 下所有 per-workdir store：逐个打开 libsql db、分页列出每个 store 的全部 session（含普通 tui/run 会话）、再读 `workdir` 标记文件，最后用 `model IS NULL` 启发式挑出 ts 会话。store 越多、会话越多越慢。

## Change Summary

- 新增 `crates/store/src/ts_registry.rs`：`TsRegistry`（`<data_root>/ts.db`，WAL + Mutex 串行化，同 `LibsqlStore` 模式）——`ts_sessions(id/workdir/store_dir/created_at/updated_at/title/preview)` + `meta(migrated=1)`；`upsert/list/get/delete/is_migrated/mark_migrated` 全幂等。
- 重写 `crates/cli/src/ts/registry.rs`：`open_registry()` 打开唯一索引并在缺 `migrated` 标记时执行**一次性迁移**（复用旧 scan 代码，把各 store 中 `model IS NULL` 会话导入，marker 缺失行 workdir=NULL 保留 `(unknown)` 语义；幂等 upsert + 末尾打标记，崩溃安全）；`register()` 取代 marker 文件写入。
- `crates/cli/src/ts/actions.rs`（855→760 行）：`ts_list` 一次索引查询 + tmux；`ts_resume`/`ts_cleanup`/`ts_delete`/`ts_start` 全部改为 registry 读写；`sync_live_workdirs` 仅当行缺失或 workdir 为空时写（`ts -l` 稳态零写入）；`is_ts_owned`/`is_registered` 启发式随扫描下线。
- 新增 `crates/tui/src/ts_mirror.rs`：`TsMirrorStore` 包装 `Arc<dyn Store>`，把 TUI 内 ts 会话的 title/preview/delete 镜像进 registry（`model IS NULL` 首次持久化识别、preview 前 80 字符一次性写入、title patch 镜像、delete/clear-other 传播）；镜像写一律 best-effort 不向上传播错误。`app_bootstrap` 仅在 `ts.db` 已存在时包裹，纯 tui/run 零影响。
- 帮助文案同步：不再"来自所有 store"，改指 central ts registry。
- 新增快速上手文档 `docs/quickstart.md` / `docs/quickstart.en.md`（README 致谢区同步）。

## Impact Surface

- `ts -l` 从「N×LibsqlStore::open + N×全量分页 + N×marker 读」变为「1×TsRegistry::open + 1×索引查询（+ 稳态零写入）」。
- 会话正文仍在各 workdir store：`session show`、`/task` picker、export、`tui`/`run` 全部零变化。
- 行为修正：`/model` 切换后会话不再从 `ts -l` 消失（旧 `model IS NULL` 启发式误判；registry 行与 store 行解耦）。
- 已知取舍：跨 store 重复 id 以 `INSERT OR REPLACE` 收敛（原 `-d` 报歧义，属病态数据）；仅 TUI 内更新镜像 registry，headless `run --session` 改标题的极端场景 registry 不更新（展示仍可用）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| upsert/get/list 往返 | `upsert_get_list_roundtrip` | `crates/store/src/ts_registry.rs` |
| 幂等替换（INSERT OR REPLACE） | `upsert_is_idempotent_and_replaces` | `crates/store/src/ts_registry.rs` |
| delete 只删目标行 | `delete_removes_only_the_target` | `crates/store/src/ts_registry.rs` |
| meta 标记往返 | `meta_marker_roundtrip` | `crates/store/src/ts_registry.rs` |
| 磁盘重开可读 | `on_disk_open_is_reopenable` | `crates/store/src/ts_registry.rs` |
| register 往返 + delete | `register_roundtrip_then_delete` | `crates/cli/src/ts/registry.rs` |
| 迁移只导入 `model IS NULL`（普通会话排除、marker 缺失 workdir=NULL） | `migration_imports_ts_owned_sessions_only` | `crates/cli/src/ts/registry.rs` |
| 迁移幂等 + 崩溃安全 | `migration_is_idempotent_and_crash_safe` | `crates/cli/src/ts/registry.rs` |
| 分页保留（超单 store 上限） | `list_all_sessions_paginates_past_store_limit` | `crates/cli/src/ts/registry.rs` |
| bare `ts` 恒新建 tmux session | `bare_ts_always_builds_new_tmux_session_command` | `crates/cli/src/ts/actions.rs` |
| 显式 attach 目标三分支（bare/存在/非 live） | `explicit_attach_target_bare_ts_returns_none` / `explicit_attach_target_session_exists_attaches` / `explicit_attach_target_session_not_live_returns_none` | `crates/cli/src/ts/actions.rs` |
| 前缀解析：live+registry 唯一 ID / 歧义拒绝 / stopped registry 行 | `managed_target_resolves_list_prefix_full_id_and_tmux_index` / `managed_target_rejects_ambiguous_prefix` / `managed_target_resolves_stopped_registry_prefix` | `crates/cli/src/ts/actions.rs` |
| `-l` 图例不再含已下线 flag | `list_legend_has_no_removed_flags` | `crates/cli/src/ts/actions.rs` |
| 三态分类（attached/live/stopped） | `classify_three_states` | `crates/cli/src/ts/actions.rs` |
| 按 workdir 升序 + created 倒序 | `sort_by_path_then_created_desc` | `crates/cli/src/ts/actions.rs` |
| union 行 = live tmux + registry stopped（显式排除 never-started seed / 未注册） | `build_rows_unions_live_tmux_and_registered_stopped` | `crates/cli/src/ts/actions.rs` |
| cleanup 目标 = 死 registry 行按 store 分组 | `cleanup_targets_are_dead_registry_rows_grouped_by_store` | `crates/cli/src/ts/actions.rs` |
| 时间戳毫秒 | `now_ms_is_milliseconds` | `crates/cli/src/ts/actions.rs` |
| ts 会话注册（title/store_dir） | `ts_session_is_registered_with_title_and_store_dir` | `crates/tui/src/ts_mirror_tests.rs` |
| 普通会话跳过不注册 | `plain_session_is_not_registered` | `crates/tui/src/ts_mirror_tests.rs` |
| 首条用户消息一次性写 preview | `first_user_message_writes_preview_once` | `crates/tui/src/ts_mirror_tests.rs` |
| 未知会话永不镜像 | `unknown_session_never_mirrors` | `crates/tui/src/ts_mirror_tests.rs` |
| title patch 仅镜像已存在行 | `title_patch_mirrors_existing_row_only` | `crates/tui/src/ts_mirror_tests.rs` |
| delete 反注册 | `delete_session_unregisters` | `crates/tui/src/ts_mirror_tests.rs` |
| clear-other 修剪 registry | `clear_other_sessions_prunes_registry` | `crates/tui/src/ts_mirror_tests.rs` |
| maybe_wrap 仅 `ts.db` 存在时包裹 | `maybe_wrap_gates_on_existing_registry` | `crates/tui/src/ts_mirror_tests.rs` |

- 全量回归：`cargo test --workspace` → **2338 passed / 0 failed**（145 个测试二进制，`EXIT=0`）。数字取自本批次工作树当次实跑（22:51/22:53/22:58 共 4 次，全部 2338/0）；评审后 act mode 复跑 1 次仍 2338/0。
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告（`EXIT=0`；修复 1 处 `needless_borrow` 后复跑确认）。
- build：`cargo build --workspace` → 编译干净。
- 行数：`actions.rs` 760 ≤ 800；`registry.rs` 292 ≤ 800；`ts_registry.rs` 306 ≤ 400；`ts_mirror.rs` 292 ≤ 400；`ts_mirror_tests.rs` 197 ≤ 400。
- 手工冒烟：真实数据根首次 `ts -l` 触发一次性迁移（5 个旧 ts 会话导入 registry），二次运行无重复；`ts.db` 创建于 `<data_root>`，legacy `workdir` marker 零写入；tmux live 会话仍正常展示。

## Related Docs

- 快速上手：`docs/quickstart.md`、`docs/quickstart.en.md`
- `crates/cli/src/ts/registry.rs`、`crates/cli/src/ts/actions.rs`、`crates/tui/src/ts_mirror.rs`、`crates/store/src/ts_registry.rs`
