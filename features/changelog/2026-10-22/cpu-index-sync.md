Commit: c8a5950cb69b4e73e3edb27027f3cdd316c6fa5b

# 降低索引同步与 Brain 上报开销

## 行为

- Server 复用五列执行索引的不可变归属；首次尚无索引时校验冻结请求，正常校验不解析大 JSON。原生受理和删除在事务内维护归属约束。
- Node 与 Server 显式协商增量，未变化清单只发送负载。首次、重连和每 10 分钟校准仍发全量；提交成功后才推进基准，失配重连，旧对端保持全量。
- 收齐报告、写库和接收证据提交分开处理。增量删除仅表示清单成员移除，不删除历史或释放活动预留；首次缺失恢复仅用于全量。
- Runtime 在加载、成功保存和删除时维护 Brain 候选集合，只提取必要字段，避免每轮复制全部历史；持久确认、嵌套终态、settle 和 64 帧轮转保持原语义。
- 协商增量时 Brain 与索引提交独立发送；慢索引写入期间继续心跳、RPC、负载和 Brain 重放。

## 兼容与上线顺序

最终实现不增加共享数据库结构。首次发布曾尝试附加归属表及休眠版本表，导致运行中的旧 Runtime 遇到结构变更错误；失败记录保留，后续改为复用已有数据。已建的附加表不删除，避免再次触发旧连接失效。双方同意后才发送增量，协议版本仍为 10。已有任务保留 Runtime 归属；后续新任务切到新 Runtime。

Ontology 核对：本次为内部存储、报告协议及上报读取优化，没有新增用户能力、CLI/API 入口或输入输出要求；共享目录中的节点和执行能力描述保持有效。

## 测试覆盖

| 功能 | 用例 | 文件 |
| --- | --- | --- |
| 原生写入始终保持归属不可变 | `native_preparation_preserves_indexed_owners_before_and_after_acceptance` | `crates/store/src/fleet/ownership/tests.rs` |
| 首次归属校验、64 位时间、整批回滚、后续不读正文 | `first_report_checks_frozen_owner_and_rolls_back_the_whole_batch` | 同上 |
| 共库基准、故障回滚、历史保留 | `delta_commit_fence_survives_other_server_reports_and_rolls_back_failures` | `crates/store/src/fleet/report/tests.rs` |
| 2 万索引空闲、单条变化、移除、周期全量 | `twenty_thousand_idle_indexes_send_nothing_and_only_commits_advance_base` | `crates/node/src/fleet/client/delta/tests.rs` |
| 提交等待期间 Brain/RPC、空闲无索引帧 | `delta_waits_for_commit_while_brain_and_rpc_continue_and_idle_has_no_report` | `crates/node/src/fleet/client/tests/incremental.rs` |
| 实际 WebSocket、共库变化、最终确认 | 两项 `incremental` 测试 | `crates/control/tests/node_keepalive/incremental.rs` |
| 候选保存、删除、重启、嵌套、无关历史 | 三项 reporting 测试 | `crates/worker/src/journal/reporting_tests.rs` |
| 超过 64 帧轮转与确认后重启 | `terminal_candidates_rotate_past_sixty_four_and_acks_survive_restart` | `crates/worker/tests/brain_outbox_candidates.rs` |
| Host 索引与 Brain 分离、旧协议串行读取 | `incremental_indexes_are_independent_of_brain_but_legacy_collection_stays_serial` | `crates/agent/src/host/tests/read_reports.rs` |
| 大冻结请求重复解析对比 | `warmed_ownership_projection_avoids_large_assignment_json_work` | `crates/store/src/fleet/report/performance.rs` |

## 实际验证

仅运行改动和直接影响链路的定向测试，未运行全量测试。以下为第一轮实现的历史验证记录，结构兼容修正后的验证见 [源增量记录](cpu-source-reporting.md)。以下记录实际命令与目标结果；复测与先前范围有重叠，不累加成总用例数。

| 实际命令 / 目标 | 结果 |
| --- | --- |
| `cargo test -p opencoder-core --lib fleet::report` | 2 passed |
| `cargo test -p opencoder-store --lib fleet::ownership` | 2 passed |
| `OPENCODER_CPU_BENCH=1 cargo test -p opencoder-store --lib fleet::report -- --nocapture` | 4 passed，含合成性能比较 |
| `cargo test -p opencoder-store --lib fleet::report::tests` | 提交代次处理修正后 3 passed |
| `cargo test -p opencoder-store --lib fleet::handoff` | 17 passed |
| `cargo test -p opencoder-store --test fleet_report --test fleet_schema` | 4 + 6 passed |
| `cargo test -p opencoder-node --lib fleet::client` | 14 passed；握手夹具改为 Callback 实现后单独复测 `fleet::client::tests::incremental`，1 passed |
| `cargo test -p opencoder-control --lib transport::` | 35 passed |
| `cargo test -p opencoder-control --lib transport::handoff_report` | 新增共库接管竞争用例后 3 passed |
| `cargo test -p opencoder-control --lib transport::hub::tests::delta_membership_preserves_unmentioned_acceptances_and_reservations` | 1 passed |
| Control 集成目标 `admission`、`catalog_guard` | 同一回归命令中的前两个目标分别 4、32 passed；后续传输目标超时后修正夹具并单独复测 |
| `cargo test -p opencoder-control --test node_keepalive --test ontology_catalog --test release_handoff` | 9 + 5 + 5 passed；最后提交代次修正后再次运行 `node_keepalive`、`release_handoff`，9 + 5 passed |
| `OPENCODER_CPU_BENCH=1 cargo test -p opencoder-worker --lib journal:: -- --nocapture` | 8 passed，含 1 GiB 无关历史的比较 |
| `cargo test -p opencoder-worker --test brain_outbox_candidates --test brain_scheduler_v4 --test brain_nested --test brain_guidance_e2e --test brain_server_restart --test brain_report_stability --test brain_dispatch_failures --test brain_closed_loop` | 20 passed，包含实际原生容器执行，使用本次代码构建的专属 rootfs |
| `cargo test -p opencoder-agent --bin opencoder-agent host::tests::read_reports` | 原有只读修订号用例通过；修正 Runtime 必须先注册 staged 再激活的测试准备后，单独运行新增并发用例，1 passed |

构建与环境：`bash scripts/prepare-dag-rootfs.sh /tmp/opencoder-cpu-sync-20261022/rootfs` 成功；集成测试使用该 `DAG_TEST_ROOTFS`，清除仅测试进程的代理变量。日志与结果保存在本机 `/tmp/opencoder-cpu-sync-20261022/`，不纳入 Git。

静态检查：`cargo clippy -p opencoder-core -p opencoder-store -p opencoder-control -p opencoder-node -p opencoder-worker -p opencoder-agent --all-targets -- -D warnings` 通过，零警告。首次检查发现测试中的多余复制，改为借用后通过；新增握手夹具实现上游 Callback，不新增 lint 豁免。触及的 Rust 文件通过 `rustfmt --check`；`git diff --check`、新增文件大小和新增文档链接检查通过。

第一轮隔离回归完成时尚未发布。后续发布与恢复记录见 [源增量记录](cpu-source-reporting.md)；线上 CPU 与延迟需要匹配负载复测。

合成性能验证（Linux，本机 debug 构建）：

- 20,000 条索引、4,000 份冻结请求，正文合计 524,288,000 字节；三轮旧归属 JSON 查询累计 13,684 ms、1,368 CPU tick，新完整归属与状态校验累计 1,693 ms、169 CPU tick，CPU 降低约 87.6%。对比旧侧仅包含最重的归属查询，新侧包含整组报告 SQL。
- 1,024 条无关 journal 记录，正文合计 1,073,741,824 字节；旧整体复制 629 ms，新候选字段提取 100 次采样 P95 小于 1 μs。该数字不含 HTTP、相关 Brain 数据库查询或 journal 初次加载，不代表线上接口耗时。
- `OPENCODER_CPU_BENCH=1` 启用上述规模；正常定向回归使用小型合成数据。没有使用实际任务正文或凭据。

测试准备中发现并记录：准入和多线程传输夹具会读取宿主配置，引发后台目录扫描和超时；补齐生命周期内配置隔离及显式临时资源根后，相关用例通过。原生镜像来自旧提交时，版本核验正确拒绝运行；使用当前代码重建专属 `DAG_TEST_ROOTFS` 后继续回归。隔离测试清除了代理环境，服务配置未改。

兼容边界：本次仅受管 Host 请求增量协商。旧 Server 对直连 Worker 不保存报告提交序号，无法防止迟到旧报告覆盖状态，因此直连 Worker 继续周期性全量；候选提取优化仍生效。Server 拒绝非 Host 代次启用增量。
