Commit: 92b4ec156acd78b62031f257c3099863b6cda6b3

# DAG 定义更新时间与测试目录清理

控制面的 DAG 保存接口此前只持久化 `id/name/spec`，导致列表和详情缺少时间字段，页面的「更新时间」始终显示空值。现在由服务端记录毫秒级 `created_at`、`updated_at`：编辑保留创建时间，每次保存推进更新时间；没有历史时间字段的旧定义从下一次保存开始记录。非法请求不改变已保存内容或时间，客户端不能覆盖时间字段。同名保存复用跨进程锁，支持平滑发布期间的多个 Server。

本次按用户指定清单执行一次性线上清理：删除 6 条大脑能力及其示例输入、向量、绑定，删除 8 个 DAG 定义，仅保留 `static-auto-test-pingce-0bdf3c65d217`、`viking-dependency-analysis`、`regression-test`、`eval-diagnose`。清理前备份保存在服务器本地；保留定义的工作流内容经过前后比对。清理没有加入启动或发布逻辑。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 创建/编辑时间、列表与详情一致、拒绝客户端时间覆盖、非法保存不修改记录 | `dag_save_tracks_update_time_and_preserves_creation_time` | `crates/control/tests/e2e/teams_dag_defs.rs` |
| 旧定义兼容、并发保存保留创建时间 | `legacy_dag_gets_timestamps_on_save_and_concurrent_edits_keep_creation` | `crates/control/tests/e2e/teams_dag_defs.rs` |
| 准入等待上报占用的锁时，上报仍可推进 | `admission_keeps_inflight_report_running_until_shared_lock_is_released` | `crates/node/src/fleet/client/tests.rs` |

- 修复前：新增时间回归测试因响应缺少 `created_at` 失败。
- 定向回归：`cargo test -p opencoder-control --test e2e teams_dag_defs`，10 passed。
- 静态检查：`cargo clippy --workspace --all-targets -- -D warnings`，通过。
- 最终代码版本 `92b4ec15`：`cargo test --workspace -j 12 --no-fail-fast -- --test-threads=4`，5,239 passed / 0 failed / 6 ignored（原有跳过项）；399 个测试结果块。完整输出保存在 `/var/lib/opencoder-platform/backups/test-catalog-cleanup-20260916-091939/full-tests-complete.log`。
- 最终代码版本的 `cargo build --workspace` 与全量 clippy 均通过；前端 695 项测试、发布脚本 50 项测试通过。
- 测试环境排查：共享构建盘 I/O 曾拖慢输出与 SQLite fsync；未指定用户的 systemd 测试服务缺少登录环境，导致严格输出字节断言失败。最终全量回归使用独立的临时内存文件系统及 `User=root`，未修改断言或新增忽略项。

## 发布验证发现的控制连接互锁

真实平滑发布验证中，新 Host 的状态查询挂起，准入同步无法完成。连接内直接轮询上报 future 时，串行处理准入会暂停上报；如果准入等待上报持有或已排队的锁，两者无法继续。上报采集改由连接持有的 `JoinSet` 独立推进；同一连接仍只采集一个报告，准入顺序保持不变，连接结束会取消采集。

新增 `admission_keeps_inflight_report_running_until_shared_lock_is_released` 回归测试：先让上报持有共享锁，再发准入请求并释放上报等待条件，要求准入成功返回。修复前稳定超时；同时复用既有断连取消采集测试。

## 线上验收

- 已发布 `92b4ec15`；Server、Host 与四个本机程序的正式构建版本一致。安装器曾因单独覆盖的未提交 CLI 构建拒绝安装；备份该程序并恢复受管理的启动入口后，正式发布成功。
- 真实模型依赖链和 WASI 任务跨发布完成，原 Runtime、模型 shell 与 NFS 进程保持连续；61 次切换期间提交无失败，最大调度延迟 89 ms，SSE 恢复约 138 ms。
- 15 分钟观察完成，171 次验证通过。页面实际保存确认创建时间不变、更新时间递增，列表与绝对时间提示正常，能力库为空，保留四条定义的 spec 与备份一致。
- `eval-diagnose`、`regression-test` 仍为已退役的 `runner` 定义，按原样保留，没有自动迁移或补写历史时间；另外两条定义已验证成功保存与时间更新。
- 后续用户要求的两条存量定义迁移已在[当前 Agent 执行适配](dag-agent-adaptation.md)中完成，包含真实执行与全部四条更新时间的验证。
- 清理备份、完整测试输出、页面截图和最终回执保存在服务器本地 `/var/lib/opencoder-platform/backups/test-catalog-cleanup-20260916-091939/`，发布验收回执为 `/var/lib/opencoder-platform/acceptance/release-live-a2581e142144b6a0/result.json`。

## 相关记忆

- [控制面定义与保存边界](../../../agents/control/index.md)
- [节点报告与准入通信](../../../agents/node/index.md)
- [Agent 平台的 DAG 定义管理](../../agent-platform/index.md)
