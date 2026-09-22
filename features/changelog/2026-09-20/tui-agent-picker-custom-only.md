Commit: 2c7c7e5918c791b93e63b5346f0f9420754c7b24

# TUI `/agent` 选择器只展示自定义 Agent

`/agent` 菜单移除内置运行时角色，执行与计划模式继续通过 `/act`、`/plan`
切换。菜单沿用自定义 Agent 目录、名称排序、描述和模糊搜索；选择后仍填入
`/agent <name> `，通过现有提交路径切换并持久化。

- 排除全部内置名称，即使磁盘存在同名注册卡也不展示。
- 没有自定义 Agent 时显示 `no custom agents available`；搜索无结果时显示
  `no matching agent`。空列表按 Enter/Tab 不切换 Agent。
- 底层角色解析、手输控制命令兼容性及 Web 入口保持原有行为。
- 文件系统目录用例移到独立集成测试，菜单单元测试只使用内存数据。

## 2026-09-22 交付边界收敛

用户确认本轮按菜单需求闭环：只验收自定义目录选择器、空态、搜索、选择、
`/act`、`/plan` 与会话恢复。下文平台发布和性能治理记录保留为历史证据，
平台三个 1 秒切换指标失败不再作为菜单需求门槛，也不因此改写为平台验收通过。

当前安装版本为 `565c0eae`。菜单功能及配置目录隔离实际在 `2c7c7e59` 引入；
原文顶部的 `363c826e` 对应 Brain 页面改动，本轮已通过 Git diff 纠正。
菜单与直接关联路径的 20 个定向测试证据已核对，当前 TUI、Core 和模式切换源码
与测试提交 `5ccaab33` 一致。后续 Session 改动仅涉及搜索，Store 改动仅涉及 Fleet；
没有把同批平台的其他测试数量计入菜单验收，也没有新跑完整回归。

本轮安装验证分别重新打开无匹配菜单测试 Enter、Tab，并增加自定义 agent 的
退出恢复检查。首轮脚本在发送退出键后立即关闭终端，日志出现 stderr I/O 错误，
该观察窗口已作废并保留证据；改为等待 TUI 正常退出并保存每次进程日志后，
9 项交互再次通过，7 次进程日志均无 ERROR 或 panic。

随后并发平台发布使观察窗口在约 195 秒时失效；该作业在回滚基线就绪及任务
验证失败后留下 `f2a72f67`。本轮取得发布互斥锁，用已验证包恢复 `565c0eae`，
并在同一把锁下重新执行菜单观察，避免用跨版本窗口签收。

最终完成约 903 秒版本绑定观察：起点、中段、终点各 9 项交互全部通过，
21 次进程日志无 ERROR 或 panic，测试会话聊天记录内容不变。安装二进制哈希
与发布 manifest 一致，观察全程版本保持 `565c0eae`。菜单需求验收完成；
平台连续性失败仍独立保留，不包含在本次签收结论内。
证据目录：`/var/tmp/opencoder-agent-menu-closeout-20260922/menu-closure`。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 空目录不补入内置角色 | `empty_registration_has_no_builtin_cards` | `crates/tui/tests/agent_menu_catalog.rs` |
| 自定义卡排序及描述 | `custom_cards_keep_sorted_names_and_existing_descriptions` | 同上 |
| 排除内置同名目录 | `builtin_named_directories_never_enter_the_catalog` | 同上 |
| 空列表确认不切换 | `enter_and_tab_close_empty_results_without_picking` | `crates/tui/src/agent_menu_tests.rs` |
| 空目录与搜索无结果提示 | `popup_distinguishes_no_custom_agents_from_no_search_matches` | 同上 |
| 选择自定义卡并持久化 | `picker_pick_fills_control_head_and_switches` | `crates/tui/tests/agent_mention_flow.rs` |
| `/act`、`/plan` 切换及恢复 | `plan_switch_persists_and_survives_resume`、`act_roundtrip_back_is_persisted`、`switch_never_folds_or_records` | `crates/tui/tests/agent_switch_persist.rs` |

## 2026-09-20 首次实现验证记录

以下保留首次实现时的结果，不代表后续发布版本的验证状态。2026-09-22 用户
明确要求只测试修改点和直接关联路径，后续交付按该范围验证。

- 菜单单元测试：`cargo test -p opencoder-tui --lib agent_menu::tests`，8 passed。
- 目录、切换与模式持久化集成测试：`cargo test -p opencoder-tui --test
  agent_menu_catalog --test agent_mention_flow --test agent_switch_persist -j 4`，
  3 + 3 + 3 passed。
- 全部 TUI 单元测试：1720 passed / 0 failed；上述三组集成测试在后续工作区
  回归中也全部通过。
- 全量 clippy：`cargo clippy --workspace --all-targets -j 4 -- -D warnings`，通过。
- 修改的 Rust 文件 rustfmt 检查及 `git diff --check`，通过。
- 全量构建：`cargo build --workspace -j 16`，通过；本轮校验设置
  `CARGO_PROFILE_DEV_DEBUG=0`，减少调试符号造成的链接 I/O。
- 全量回归：`cargo test --workspace --no-fail-fast -j 16 -- --test-threads=8`，
  exit 101，5 个测试目标未通过。输出中的完整 `test result` 汇总行累计
  **5426 passed / 3 failed / 7 ignored**；此计数不包含未产生汇总行的中断目标
  和文档测试编译错误。当前未满足仓库全量回归 gate。

全量回归使用独立构建目录 `/data00/rust-build/cargo/myagent-check`，设置
`CARGO_PROFILE_DEV_DEBUG=0`，标准输入为 `/dev/null`。执行期间共享工作区另有
Brain/Control 等模块的变更；上述 clippy、构建结果仅代表各自执行时的代码状态。

### 未通过项

| 测试目标 | 结果 |
| --- | --- |
| `opencoder --test dag_e2e` | `dynamic::runc_dynamic_agent_and_wasm_read_isolated_copies_and_argv` 等待 `dag-dynamic-runc` 终态超过 180 秒。 |
| `opencoder-dag-runtime --test dynamic` | `persistence::scheduling_persistence_failure_cancels_and_drains_live_instances` 和 `recovery::timeout_is_per_instance_and_user_cancel_is_run_cancellation` 返回 `Elapsed`。 |
| `opencoder-agent --bin opencoder-agent` | `host::tests::three_runtime_versions_keep_live_model_calls_and_global_fifo` 持续等待；目标运行 624 秒后以 SIGTERM 中断。单独复核也未在 30 秒上限内结束。栈位于 `dispatch_locked` → `enqueue_capacity` → 临时数据库 WAL 的 `fsync`，具体根因尚未确认。 |
| `opencoder-brain --doc` | E0425：无法解析 `SchedulerPlan` 类型。 |
| `opencoder-control --doc` | E0425：无法解析 `SchedulerPlan`；E0432：无法导入 `opencoder_brain::activation::configured_request`。 |

上述失败属于首次实现时的工作区快照；原始失败记录继续保留。后续交付遵循
用户指定的定向测试范围，不将完整工作区回归作为本次交付门槛。本改动未增加
忽略测试或放宽断言。

原始结果保留在 `/tmp/opencoder-agent-menu-workspace-tests.log`，检查回执在
`/tmp/opencoder-agent-menu-gates.json`；独立 TUI 单元测试日志为
`/tmp/opencoder-agent-menu-tui-lib-final.log`。

相关记忆：[TUI 模块](../../../agents/tui/index.md)。

首次实现阶段未发布。

## 2026-09-22 定向交付进展

- `5ccaab33`：39 项 Rust、20 项前端定向测试通过。TUI 相关 20 项覆盖菜单
  单元测试、目录读取、选择流程、模式持久化及启动覆盖；其余覆盖同批发布的
  Brain 层级调度历史和关联接口。完整工作区回归已按用户要求停止。
- `736b729f`：发布观察发现原生搜索超时后后台扫描仍可能继续，补充二进制跳过、
  行缓冲与结果容量限制、取消传播；22 项搜索与工具契约测试通过。
- `0d0c0182`：连续性验收发现任务索引同步占用全局写锁，改为批量校验和仅更新
  变化状态；11 项索引报告、归属、恢复和版本交接测试在隔离工作区通过。
- `40c16acc`：进一步定位到待恢复派发查询扫描历史大字段，改为先筛选待恢复回执，
  再读取对应任务内容；增加索引与请求一致性校验，异常任务类型明确报错。
  15 项关联检查通过，包含 9 项报告与交接复验。同一只读
  数据快照上，新旧查询结果一致，耗时由约 0.12 秒降至 0.02 秒。
- `023e141c`：就绪检查逐页扫描全部历史任务的开销被只读快照复现，改为单次
  聚合统计并保留字段类型及枚举校验；3 项统计、4 项准入、4 项排空测试和
  定向 clippy 通过。同一快照 23,792 条记录，查询耗时由约 676 毫秒降至 22 毫秒。
- `a533441d`：为慢请求增加准入、资源准备、持久化和派发阶段耗时日志，仅记录
  任务 ID、阶段名与耗时；10 项准入、重试及配置预检关联测试通过。
  候选版与回滚基线 `2683b903` 使用相同计时源码，仍按原 1 秒门槛验收。
- `4df57333`：容量统计、先进先出资格查询和 Runtime 容量同步使用已有的
  未完成任务部分索引；3 项容量、5 项交接、1 项重试测试通过，不增加表或索引。
- `565c0eae`：Host 清单同步改为批量读取不可变任务归属，新发现任务在事务内
  再次校验并原子写入。4 项新用例及 9 项容量、交接、Host 关联检查通过，
  Store 和 Host 定向 clippy 通过。18,921 条归属的只读查询对比从约 248 毫秒
  降至 18 毫秒，结果一致。原测试提交为 `883d5a54`，四个改动文件与合并提交
  及同等源码的回滚基线 `45d0e4f4`、`f2a72f67` 字节一致。合并版本再次通过
  3 项容量、1 项交接及 1 项三个 Runtime 共存检查。
- 当前选用的定向验证为 135 项次 Rust（108 个独立用例及 27 项关联复验）、20 项前端测试，
  各轮保留实际测试 commit，
  通过源码差异和哈希核对复用未变化部分的证据。未将其他会话尚未提交的
  `private_files` 开发改动计入本次测试或发布。

`40c16acc83fe7c648472b00e5d27b992fc864b53` 已部署，历史记录及六类任务展示验证通过。
生产原生搜索完成 2 次实际调用，二进制跳过和超长文本报错通过，Runtime 峰值约 41 MiB。
随后实际回滚到 `0d0c0182` 并重新发布；757 个请求均成功且任务完成，但全窗口最大受理
耗时为 1.510 秒、受理间隔 1.610 秒、调度间隔 1.780 秒，未通过原定 1 秒门槛。
两次受理超限均发生在回滚版本 `0d0c0182`；当前版本处理的 354 个请求最大受理
耗时为 0.883 秒。分版本分析仅用于定位，不替代整个切换窗口的验收。
失败回执保留在 `/tmp/opencoder-outbox-continuity-closure/agent-menu-continuity-latency-review.json`。
更换为含搜索、报告和待派发查询修复的 `d9b366a6` 回滚基线后，第二个窗口
740 个请求零失败，但最大受理耗时仍为 2.409 秒，受理间隔 2.509 秒、
调度间隔 2.487 秒。该失败保留在 `/tmp/opencoder-brain-final-closure/agent-menu-continuity-latency-review.json`，
随后完成上述 `023e141c` 就绪统计修复。
`023e141c` 与含同等统计修复的回滚基线 `902cd95b` 已完成实际部署，
生产就绪检查降至约 27 毫秒。随后切换窗口的 935 个请求均成功且任务完成，
但有一个基线任务从创建索引到启动等待 1.037 秒，使最大受理耗时达到 1.077 秒、
受理间隔 1.177 秒、调度间隔 1.176 秒，仍未通过原定门槛；其余最慢请求约 244 毫秒。
该失败及阶段时间证据保留在 `/tmp/opencoder-readiness-continuity-closure`。
随后为定位尾部延迟加入上述阶段日志。候选版首次构建命中了回滚版缓存，提交号
核验将其拒绝；保留失败日志并清理该构建缓存中的 core 产物后，重新构建和核验通过。
`a533441d` 与 `2683b903` 的切换窗口共有 910 个请求，全部成功且任务完成。
最大受理耗时 0.990 秒，但受理间隔 1.090 秒、调度间隔 1.237 秒，仍未通过原定门槛。
新增日志记录到容量同步阶段等待 878 毫秒；同期宿主 I/O 压力较高，仍需区分
任务记录锁等待与共享容量数据库等待。日志提取已兼容 journald 的字节数组消息，
原始失败和阶段证据保留在 `/tmp/opencoder-admission-timing-closure`。
`565c0eae` 已完成部署，历史接口、历史浏览器页面、六类任务展示和原生搜索均通过。
与 `f2a72f67` 的实际回滚及重新部署窗口共 689 个请求，全部成功且 DAG 完成，
最大受理耗时 1.086 秒、受理间隔 1.186 秒、调度间隔 1.460 秒，仍未通过原定门槛。
最慢请求的 DAG 在请求发出约 140 毫秒后启动，HTTP 受理响应在启动后约 946 毫秒返回；
对应 Runtime 没有超过 250 毫秒的准入阶段日志，需要继续核查返回链路及 Server 回执持久化，
不能据此把问题归因于已修复的容量查询。完整失败数据与只读诊断保留在
`/tmp/opencoder-host-capacity-closure`。
以下为交付边界收敛前的阶段状态：生产为 `565c0eae`，安装后二进制的 8 项 TUI 检查已独立通过，包括空列表、无匹配、
自定义项及 `/act`、`/plan` 持久化；这不替代正式发布窗口验收。当时仍需通过平台连续性门槛和 900 秒观察，
该阶段未通过平台交付门槛；菜单最终验收以本文“交付边界收敛”记录为准。
