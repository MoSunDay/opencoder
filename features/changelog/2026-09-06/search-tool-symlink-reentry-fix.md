# search 工具 symlink 重入死循环修复（一核打满根因）

线上 PID 217686（opencoder TUI）一个 tokio worker 连续打满一核近一小时。
gdb 附着该线程，栈顶是 `open()` 一个无限嵌套路径：
`/proc/1682/task/1682/root/proc/3438/task/3438/root/sys/.../subsystem/devices/i2c-3/...`。
根因在 `crates/session/src/tools/search.rs`：`WalkBuilder` 开了
`follow_links(true)`，而原注释声称「ignore walker 自带环检测所以安全」——
该断言错误。walkdir 的环检测只识别「目录出现在自己的祖先链上」；
`/proc/<pid>/root` 每跳都解析出**不同**目录（目标同为 `/`），sysfs
`subsystem/devices` 链每跳拼接新路径，祖先检测全部放行，遍历按指数级
广度展开、永不终止。会话中 LLM 以 `/`（或 `/proc`/`/sys`）为根调用
search 即触发。

独立复现实证（ignore 0.4.31，16 层二叉 symlink 扇出，17 个物理目录）：
旧逻辑访问 **393,196 个条目、耗时 42s**；真实 `/proc`（数千 pid，每个
pid 的 `root` 都是一次全新入口）即等效无限深度 → 一核永久打满。

## Change Summary

- **保留 follow、禁止重入**（用户语义：可以 follow，但不能重入死循环）：
  `WalkBuilder::filter_entry` 注入 `dir_first_visit` 守卫——按物理目录
  `(dev, ino)` 维护全局已访问集合（`Arc<Mutex<HashSet>>`，单线程 walk
  无争用；filter 要求 `Fn + Send + Sync + 'static` 故用 Mutex 内部可变），
  任一物理目录只进一次；symlink 依旧跟随（symlinked base/symlinked file
  行为不变），但已访问目录的重入路径被剪枝，遍历上界 = 物理目录数。
  非 unix 平台 cfg 退化为不剪枝（原行为）。
- 修正误导性注释：walkdir 祖先链环检测 ≠ 重入防护。
- `MAX_MATCHES` 短路与输出路径零改动。

## Impact Surface

- 仅 `crates/session/src/tools/search.rs`（+守卫函数 +4 个回归测试，
  388 行）。
- `/proc`、`/sys`、`root`/`cwd` 类链接的重入爆炸被消除；同目录多链接
  场景搜索结果由 N 份去重为 1 份（语义更正确）。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 不同跳目录的链接扇出必须终止（2^25 路径，旧代码挂死） | `search_terminates_on_distinct_hop_link_fanout` | `crates/session/src/tools/search.rs` |
| a→b→a 环必须终止且命中 | `search_terminates_on_symlink_cycle` | `crates/session/src/tools/search.rs` |
| 同目录多 sibling 链接只搜一次（重入剪枝） | `search_no_dir_reentry_via_sibling_links` | `crates/session/src/tools/search.rs` |
| symlinked 目录作为 path 仍被跟随搜索 | `search_follows_symlinked_base_dir` | `crates/session/src/tools/search.rs` |

- 全量回归：`cargo test --workspace` → 302 suites / 4440 passed / 0 failed
- clippy：`cargo clippy --workspace --all-targets -- -D warnings` → 零警告
- build：`cargo build --workspace` + `cargo build --release` → 零错误

## 部署备注

已替换 `/usr/local/bin/opencoder`（inode 交换）。正在运行的旧实例
（如 PID 217686）持有旧代码，`spawn_blocking` 中的失控遍历无法从外部
取消，需重启该实例才会释放被占核心。
