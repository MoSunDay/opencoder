Commit: (working-tree)

# 提升 `opencoder ts -l` 速度：ts 内容集中到一个 db

## Context

`ts -l`（及 `-r/-d/-c`）每次都要扫描 `<data_root>/<hash>/opencoder.db` 下所有 per-workdir store：逐个打开 libsql db、分页列出每个 store 的全部 session（含普通 tui/run 会话）、再读 `workdir` 标记文件，最后用 `model IS NULL` 启发式挑出 ts 会话。store 越多、会话越多越慢。

## Change Summary

- 新增 `crates/store/src/ts_registry.rs`：`TsRegistry`（`<data_root>/ts.db`，WAL + Mutex 串行化，同 `LibsqlStore` 模式）——`ts_sessions(id/workdir/store_dir/created_at/updated_at/title/preview)` + `meta(migrated=1)`；`upsert/list/get/delete/is_migrated/mark_migrated` 全幂等。
- 重写 `crates/cli/src/ts/registry.rs`：`open_registry()` 打开唯一索引并在缺 `migrated` 标记时执行**一次性迁移**（复用旧 scan 代码，把各 store 中 `model IS NULL` 会话导入，marker 缺失行 workdir=NULL 保留 `(unknown)` 语义；幂等 upsert + 末尾打标记，崩溃安全）；`register()` 取代 marker 文件写入。
- `crates/cli/src/ts/actions.rs`（855→757 行）：`ts_list` 一次索引查询 + tmux；`ts_resume`/`ts_cleanup`/`ts_delete`/`ts_start` 全部改为 registry 读写；`sync_live_workdirs` 仅当行缺失或 workdir 为空时写（`ts -l` 稳态零写入）；`is_ts_owned`/`is_registered` 启发式随扫描下线。
- 新增 `crates/tui/src/ts_mirror.rs`：`TsMirrorStore` 包装 `Arc<dyn Store>`，把 TUI 内 ts 会话的 title/preview/delete 镜像进 registry（`model IS NULL` 首次持久化识别、preview 前 80 字符一次性写入、title patch 镜像、delete/clear-other 传播）；镜像写一律 best-effort 不向上传播错误。`app_bootstrap` 仅在 `ts.db` 已存在时包裹，纯 tui/run 零影响。
- 帮助文案同步：不再"来自所有 store"，改指 central ts registry。

## Impact Surface

- `ts -l` 从「N×LibsqlStore::open + N×全量分页 + N×marker 读」变为「1×TsRegistry::open + 1×索引查询（+ 稳态零写入）」。
- 会话正文仍在各 workdir store：`session show`、`/task` picker、export、`tui`/`run` 全部零变化。
- 行为修正：`/model` 切换后会话不再从 `ts -l` 消失（旧 `model IS NULL` 启发式误判；registry 行与 store 行解耦）。
- 已知取舍：跨 store 重复 id 以 `INSERT OR REPLACE` 收敛（原 `-d` 报歧义，属病态数据）；仅 TUI 内更新镜像 registry，headless `run --session` 改标题的极端场景 registry 不更新（展示仍可用）。

## Validation

- 单元测试（store）：`ts_registry` upsert/get/list/delete roundtrip、幂等替换、meta 标记、磁盘重开。
- 单元测试（cli）：迁移只导入 `model IS NULL`（普通会话不导入、marker 缺失 workdir=NULL）、迁移幂等崩溃安全、register roundtrip、分页保留；actions 的 build_rows/cleanup_targets/resolve_managed_id 适配 registry。
- 单元测试（tui）：`TsMirrorStore` 注册/镜像/普通会话跳过/delete 与 clear-other 传播/maybe_wrap 存在性门控。
- `cargo test --workspace`：**2338 passed / 0 failed**（145 个测试二进制，当次实跑 `EXIT=0`）；其中本特性新增 30 项：`ts_registry` 5、`ts::actions`+`ts::registry` 17、`ts_mirror` 8。
- `cargo clippy --workspace --all-targets -- -D warnings`：零警告（`EXIT=0`；修复 1 处 `needless_borrow` 后复跑确认）。
- `cargo build --workspace`：编译干净。
- 手工冒烟：真实数据根首次 `ts -l` 触发一次性迁移（5 个旧 ts 会话导入 registry），二次运行无重复；`ts.db` 创建于 `<data_root>`，legacy `workdir` marker 零写入；tmux live 会话仍正常展示。

## Related Docs

- `crates/cli/src/ts/registry.rs`、`crates/cli/src/ts/actions.rs`、`crates/tui/src/ts_mirror.rs`、`crates/store/src/ts_registry.rs`
