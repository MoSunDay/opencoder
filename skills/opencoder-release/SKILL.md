---
name: opencoder-release
description: 完整执行 OpenCoder 发布或回滚：审查发布历史和待发布改动、全量验证、构建可追溯发布包、由 Server 信号触发独立发布作业、验证任务连续性和公共入口、完成 15 分钟观察并记录回执。用户要求发布 OpenCoder、生效当前改动、平滑升级或信号回滚时使用。
---

# OpenCoder 发布

在用户指定的 OpenCoder 仓库和部署配置上完成发布。用户已要求上线时，完成检查后直接执行，不把授权重复变成确认步骤。默认仓库 `/data00/github/opencoder`，默认配置 `/etc/opencoder/server/opencoder.json`；先核验它们属于本次目标。

## 1. 核对范围

- 阅读仓库指令，检查 `git status`、分支和最近提交；用 `scripts/platform/deploy.sh --status` 读取当前/上一版本、候选和失败记录。
- 对照当前线上发布 commit，合并并审查所有待发布修改，保留此前功能及并行工作的改动。构建必须来自干净的已提交代码。
- 新任务切到新 Runtime；已有任务、回复、重跑和子任务沿原归属执行。发布不停止调度、不重启旧 Runtime、不重启 NFS、不改凭证、不删除数据、不恢复旧数据库。
- 若已有发布未完成，先根据持久阶段和原 bundle 续跑或明确执行兼容回滚；不生成替代任务 ID，不清空候选记录。

## 2. 验证与构建

按仓库规则完成 `cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`cargo build --workspace`，以及受影响前端测试。运行：

```sh
python3 -m unittest discover -s scripts/platform/signal_tests -v
python3 -m unittest discover -s scripts/platform/rolling_tests -v
python3 -m unittest discover -s scripts/platform -p 'test_*.py' -v
python3 -m unittest discover -s scripts/acceptance/smooth_release/tests -v
scripts/platform/release/build.sh --output /absolute/path/to/new-bundle
```

使用可用的兼容 Rust 缓存和项目既有构建配置；保存完整日志。检查行数、敏感信息、SPA 漂移、发布包摘要和编译 commit。不要以零命中的过滤测试或进程启动成功充当验收。

激活前执行 `scripts/platform/deploy.sh --backup /absolute/path/to/unique-backup` 保存在线数据库备份；这是逐库备份，不是跨库同一时刻快照，不需要暂停调度。

## 3. 生效

优先让真实模型验收脚本执行发布，并使用独立 systemd 作业运行它，防止终端/旧 Server 退出中断验收。完整命令见下一节。

只需执行已验证包的正常信号发布时：

```sh
scripts/platform/deploy.sh --signal --bundle /absolute/path/to/new-bundle --wait-seconds 300
scripts/platform/deploy.sh --status
```

首次安装信号支持：当前 Server 的 `/api/admin/release` 尚无 `signal_protocol: 1` 时，先用 `deploy.sh --bundle ...` 发布支持信号的版本；绝不能直接发送 USR1/USR2 给旧二进制，其默认行为会结束进程。此后发布和回滚均可走信号。

需要分开准备和触发时：

```sh
scripts/platform/deploy.sh --stage --bundle /absolute/path/to/new-bundle
# 从 release-state.json 的 releases[current].server_unit 取得精确 unit，
# 核验该实例的 signal_protocol 为 1 后：
systemctl kill --kill-who=main --signal=SIGUSR2 <current-server-unit>
```

USR2 消费已暂存的包；USR1 回滚到上一兼容版本。Server 将操作发给本机 Host，Host 启动独立 systemd 作业；作业预热新二进制、校验探针、切换新任务归属及入口并退役旧入口。信号送达不等于发布完成。

```sh
scripts/platform/deploy.sh --signal --rollback --wait-seconds 300
```

回滚只切换新任务入口，已被新版接受的任务继续留在新版。回滚目标若不具备信号支持，后续必须先正常发布支持信号的版本。

## 4. 完成验收

对一般新包，以以下脚本完成真实 TODO 依赖链、长 WASI、持续提交、SSE 游标和 900 秒观察；它自身执行发布，因此不要提前把候选激活。通过 `systemd-run` 以独立作业启动，使用 Python 的绝对路径、仓库绝对路径、唯一 unit 和证据目录。

```sh
python3 scripts/acceptance/smooth_release/live.py \
  --config /etc/opencoder/server/opencoder.json \
  --bundle /absolute/path/to/new-bundle --signal --observe-seconds 900
```

需要验证信号回滚再发布，在两版都支持信号后增加 `--signal-roundtrip`；这会在旧长任务仍运行时执行发布 → 回滚 → 再发布。若当前 Server 尚不支持信号，首次验收省略 `--signal`。

- 确认公共入口的实例版本、真实探针完成和任务持久归属；核对旧 Runtime、工具进程和 NFS 身份未改变。
- 持续提交失败为 0；接收及调度间隔均不超过 1 秒；SSE 5 秒内自动恢复且逐游标补齐。
- 检查最终 `result.json` 为 PASS、观察至少 900 秒；检查 Server/Host/Runtime 状态及退役回收失败。
- CLI 超时不会取消独立发布作业。先查 `--status` 的 `signals.receipts`、发布阶段和对应 unit journal，判断作业仍运行、失败还是已完成。不要以超时为理由重启业务进程；不并发发布。
- 失败必须直接报告，并依据原发布 ID 续跑或兼容回滚。保留失败证据，不将重试成功改写为从未失败。

## 5. 交付

保存构建/测试日志、发布回执、真实任务和观察证据，更新本次变更的测试映射。任务完成后按仓库要求维护 repo-local-memory。核对主工作区代码和生产已发布功能一致；最后报告版本、验证结果、旧 Runtime 剩余任务、回执路径及任何尚未完成项。无需等待仍在正常执行的旧任务结束。
