Commit: 3c1222a5e61536ec96914a7edb40d246bc6665e6

# Server 信号发布与回滚

## 行为

- Unix Server 在监听前注册 USR2（发布）与 USR1（回滚），转发带版本身份的认证请求到本机 Host；操作失败保持服务和 admission。
- Host 只允许当前 Server 和当前 Host 启动预装的部署模板，实例名包含操作与来源版本。独立 systemd 作业使用固定脚本快照，旧 Server/Host 退役不终止发布。
- 作业沿用发布互斥锁、版本兼容检查、持久阶段、任务归属、实际执行探针与 Nginx 切换；启动和失败均保存回执。过期信号拒绝，切换中断只允许原操作续跑。
- `deploy.sh --stage --bundle ...` 暂存校验包；`--signal --bundle ...` 暂存、发信号并等待本次回执；`--signal --rollback` 回滚；`--status` 包含暂存包和信号回执。
- 旧二进制未声明 `signal_protocol: 1` 时禁止发送信号，先正常发布一次支持信号的版本。重复 CLI 不能改写正在触发的候选，旧回执不能冒充本次成功。
- 新增 `skills/opencoder-release/SKILL.md`，覆盖审查、全量验证、干净提交构建、在线备份、生效、真实任务验收、观察和回执。生产验收支持 `--signal` 及带新旧长任务的 `--signal-roundtrip`。

## 测试覆盖

| 功能 | 测试名 / 入口 | 文件 |
| --- | --- | --- |
| Server 认证转发与失败保持服务 | `signals_forward_authenticated_actions_and_propagate_rejection_without_retiring` | `crates/control/tests/release_signals.rs` |
| Host 固定命令、版本身份 | `only_current_server_can_start_the_two_fixed_jobs` | `crates/agent/src/host/deployment.rs` |
| HTTP 鉴权和新旧实例隔离 | `deployment_http_requires_authentication_current_host_and_current_server` | `crates/agent/src/host/tests.rs` |
| 互斥、恢复、旧版本保护、回执和不可变控制器 | `SignalTests` | `scripts/platform/signal_tests/test_signal.py` |
| CLI 参数和锁释放/候选保护 | `CliTests` | `scripts/platform/signal_tests/test_cli.py` |
| 实际 USR1/USR2、独立作业失败、重试保持服务 | `exercise` | `scripts/acceptance/signal_release/main.py` |
| 真实信号发布/回滚/再发布与任务连续性 | `execute`、`test_signal_acceptance_uses_operator_cli_for_publish_and_rollback` | `scripts/acceptance/smooth_release/transitions.py`、`tests/test_live.py` |

基线：上一轮 workspace 5,228 项通过、0 失败、6 项既有手工测试忽略。本轮信号工具 12 项、原发布工具及重新激活测试 19 项（含 24 个故障子场景）、安装/备份工具 19 项、验收夹具 4 项通过。完整 Rust 回归 5,231 项通过、0 失败、6 项既有手工用例忽略；全目标 Clippy 零警告。日志 `/var/tmp/opencoder-signal-rust-all.log`、`/var/tmp/opencoder-signal-clippy.log`。workspace 构建通过，日志 `/var/tmp/opencoder-signal-build.log`。真实 USR1/USR2、独立 systemd 作业、失败保持服务和重复信号演练通过：`/tmp/opencoder-smooth-functional/opencoder-smooth-7awvbh35/signal-result.json`。生产信号回环和完整观察已通过，见下方验收记录。

## 信号能力首次上线

- `14f594b1` 优化包完成四个二进制的编译元数据、SPA 摘要和文件摘要校验；日志 `/var/tmp/opencoder-signal-release-r1-build.log`。
- 使用原平滑协议从 `320dbbf3` 上线 `14f594b1`，发布阶段 `complete`、独立引导作业退出 0；日志 `/var/tmp/opencoder-signal-bootstrap.log`。后续验收包用于信号发布、回滚和再发布演练。
- 激活前在线备份 6 个数据库：`/var/lib/opencoder-platform/backups/pre-signal-20260915`，明确非跨库同一时刻快照。
- 校验发现本机 root 用户 inotify 实例配额耗尽：`inotify_init1` 返回 EMFILE。将 `/etc/sysctl.d/90-opencoder-inotify.conf` 的 `fs.inotify.max_user_instances` 从系统原值 128 提高为 1,024，分配及 systemd 校验复测通过；不涉及业务进程重启。

- 首次 CLI 信号检查发现本机 systemd 不支持 `--kill-whom=main`，命令在发信号前失败，生产保持原状。已使用本机支持的 `--kill-who=main`，新增 `test_systemctl_accepts_emitted_signal_arguments_without_sending_a_signal` 对生成参数执行真实 systemctl 解析检查；失败日志保留 `/var/tmp/opencoder-signal-bootstrap-signal.log`。

## 重新激活仍在退役的版本

- 再发布保留版本时，为 Server/Host 分配新的 unit 和端口，原 HTTP 请求和 RPC 继续完成，Runtime unit、数据与任务归属保留。重新激活时重置该版本的退役标记，确保下一次切换会退役本次新建的入口。
- 测试：`test_republish_keeps_old_response_instances_and_retires_each_new_activation`、`test_reactivation_port_exhaustion_keeps_the_original_record`、`test_interrupted_reactivation_reuses_its_new_instance_identity`；见 `scripts/platform/rolling_tests/test_deployment.py`。发布回归共 19 项通过，仍覆盖 24 个中断子场景。

## 生产验收

- 当前版本 `3c1222a5e61536ec96914a7edb40d246bc6665e6`，上一兼容版本 `14f594b13afb020334abd009ed7c7bda8ca76724`。真实信号完成发布 → 回滚 → 再发布；三个作业都有新的成功回执。四个已安装二进制均对应当前干净提交。
- 跨发布 TODO 依赖链和长 WASI 完成；旧 Runtime、真实模型 Shell 和 NFS 在切换过程中保持原进程。新版长任务跨回滚继续执行，重新发布使用新 Server/Host 实例，Runtime 不变。
- 持续提交 165 次、失败 0；最大受理 115 毫秒，最大接收间隔约 215 毫秒、调度间隔 215 毫秒。SSE 三次重连耗时约 113/150/103 毫秒，游标与持久事件逐条一致。
- 独立 systemd 验收作业完成完整 900 秒观察，172 个实际执行探针全部完成，退出码 0。Node ID 和整机容量 20 保留，无退役失败。
- 旧 Runtime 休眠后，历史查询自动唤醒原 `14f594b1` 二进制；原 TODO 状态和已接受回执保留。旧入口和空闲 Runtime 均安全退出，版本包与数据保留。
- 最终结果：`/var/lib/opencoder-platform/acceptance/signal-20260915/live/release-live-0d16780ab6e89ca5/result.json`。测试、构建、信号回执、历史唤醒和服务状态统一归档到 `/var/lib/opencoder-platform/acceptance/signal-20260915`。

## 相关文档

- [发布操作与协议](../../../docs/smooth-release.md)
- [Agent 平台能力](../../agent-platform/index.md)
- [opencoder-release skill](../../../skills/opencoder-release/SKILL.md)
