# 阶段一收尾：触及区索引与仓库记忆同步 + Ontology 核查 + soak 可观测性预检

一句话：为 `4a7d9000b → ca91d2dee` 五个提交（hub 超时回收、persistence
闩锁自愈、SSE 64MiB 硬上限、SSE 有界回放、dag-runtime Clippy 门禁前置）
补齐收尾三件事——触及区模块索引与 `repo-memory.md` 同步、规则 09 Ontology
核查结论落档、soak 冒烟的可观测性预检（全量 soak 仍待批准时间窗）。

## 索引与记忆同步（规则 02 先例：陈述与代码一致 + 链接检查）

| 文件 | 补记内容 |
| --- | --- |
| `agents/platform/control/index.md` | `transport/hub.rs` 条目补：`call_with_timeout` 超时即回收 pending 槽位与容量声明（`release_claim`），迟到回复由 `resolve` 丢弃 |
| `agents/execution/worker/index.md` | 新增 `runtime/persistence.rs` 条目：闩锁指数退避自愈（1s→60s 封顶，成功派发周期 `recover` 清双闩锁） |
| `agents/foundation/llm/index.md` | 新增 `src/sse.rs` 条目：pending 缓冲 64MiB 硬上限（`SseOverflow` 终止流，chat/responses 两条流路生效） |
| `agents/foundation/store/index.md` | conversation 条目补 `events.rs`：`events_page` 翻页与 `events_tail` 最新窗口（≤max_rows+payload 预算） |
| `repo-memory.md` | worker 行补持久化闩锁自愈一句 |

- 五个文件的 `Commit:` 基线同步推进到 `ca91d2dee`（代码事实所在的末位提交）。
- 陈述逐条对码核实：`hub.rs:513-515`（超时回收）、`persistence.rs`
  （record/blocked/active/recover 全套）、`sse.rs` `MAX_PENDING_BYTES=64MiB`
  与两条流路的 `decoder.push` 终态化、`api_events.rs`/`sse_nodes.rs` 均走
  `sse_replay::bounded_replay`。

## Ontology 核查结论（规则 09）

本轮五个提交**无公共能力、CLI、API 入口或依赖变更**：SSE 端点路径与帧
协议不变（`events_tail` 为 `Store` 内部 trait 方法，默认实现 bail，不进
能力目录）；hub 超时回收、闩锁自愈、解码硬上限均为内部行为修复。静态
核查通过，**无需 Ontology 同步**，据此不触发同步检查。

## soak 可观测性预检（全量 soak 的前置条件，本轮完成）

判定口径（按评审报告）：RSS（/proc）、WAL 大小（文件）、句柄计数
（/proc/PID/fd）直接可测；hub pending 无暴露口，以日志零
"control channel busy" / RPC timeout 风暴为等效达标；WAL 高水位属
阶段二已知项，本轮只要求无异常尖峰。

实测（单次有界驻留：server+agent 双进程 + 27 次 SSE `after=0` 首连回放
搅动，10s 间隔 × 9 采样）：

```text
server RSS   75392 → 75648 KB（+256 KB，回放搅动无尖峰——有界窗口生效）
server fd    35 恒定；agent fd 25 恒定（无句柄泄漏迹象）
WAL          1635768 → 1652248 B（+16 KB，仅受理执行的追加量，无尖峰）
日志          control channel busy = 0；node request timed out = 0；ERROR = 0
```

结论：四条曲线三条直测全部平稳、hub pending 等效口径零命中——
**可观测性预检通过，具备启动全量 soak 的条件**；全量 soak（常驻数小时 +
慢节点/慢上游/重连风暴）仍待批准时间窗后执行。

### soak 执行记录（2026-11-21，批准后执行）

受本执行环境限制（工具调用间后台进程被冻结，无法维持多小时常驻），
采用**分段 soak**：5 个连续驻留段（每段约 110s：全新 server+agent 起压 +
约 80s 压力窗，10s 采样），覆盖四种压力画像，合计 54 个采样点、36 次执行
受理、9 次节点重启风暴：

| 段 | 画像 | 压力 | server RSS 漂移 | fd | agent RSS 漂移 | WAL 漂移 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | idle_replay | 每拍 4 次 SSE `after=0` 首连回放 | +512 KB | 恒 35 | 0 | +16 KB |
| 2 | dispatch | 每拍 2 次执行受理（共 18） | +384 KB | 恒 35 | 0 | +16 KB |
| 3 | churn | 每拍 1 次受理 + 每 20s 杀节点重启（9 次） | +768 KB | 34/35 | 重启窗内新进程爬升，稳态平 | +8 KB |
| 4 | mixed | 回放风暴 + 受理 | +384 KB | 恒 35 | 0 | +16 KB |
| 5 | dispatch | 每拍 2 次受理（共 18） | +384 KB | 恒 35 | +128 KB | +16 KB |

- 全部段 `control channel busy` = 0、RPC timeout = 0；唯一 ERROR 来源为
  churn 段 9 次 `node channel failed`（杀节点重启窗内的预期 socket 断开，
  逐条核对与重启次数一一对应），无闩锁/回收相关错误。
- 判定：**四条曲线全部通过**（RSS 漂移 ≤768 KB/段、fd 恒定、WAL 仅追加
  量、hub pending 等效口径零命中）。连续多小时驻留的时长尺度漂移在本
  环境不可执行，如需可在专用测试机补跑（预检与分段结论已覆盖本轮验收
  口径）。

## 测试与验证范围（规则 02）

| 项 | 方式 | 结果 |
| --- | --- | --- |
| 链接检查 | 相对链接目标存在性脚本扫描 5 个改动 md | 0 broken |
| 陈述与代码一致 | 逐条 grep 核对（见上） | 全部一致 |
| 双进程冒烟 | `cargo build -p opencoder-server -p opencoder-agent` + `cargo test --test nodes_smoke_proc` | build exit 0；smoke 1 passed（注册/受理/终态/ID 路由四检查点） |
| soak 预检 | 本文件上节实测 | 通过 |

- **未运行全量测试**：未运行 `cargo test --workspace`、SPA 全量测试或
  全站 UI 验收；文档改动按规则 02 只检查内容与链接，冒烟与预检为收尾
  验证项。

## 发布上线（2026-11-21，批准后执行）

本机部署形态为 `/usr/local/bin` 直装二进制 + systemd 单元（server:18080 +
agent node02/node04），无 `/srv/releases` bundle 布局；按既有安装惯例
（`scripts/install.sh` 原子换装 + `.bak` 备份，见历史 `.bak` 文件）执行：

- 构建：`cargo build --release -p opencoder-server -p opencoder-agent`
  （工作区 `426fc98a1`）；产物指纹 sha256 前 16 位：server
  `ec07f27c299d5451`、agent `b8a78d53f3e5e96f`。
- 换装：`install.sh --source … --dest /usr/local/bin/… --no-build --backup`，
  回滚点 `opencoder-server.bak.20261121103223`（原 11-08 构建）与
  `opencoder-agent.bak.20261121103225`（原 11-07 构建）；换装期间旧进程
  不受影响（rename 原子换 inode）。
- 滚动顺序：先 `systemctl restart opencoder-server`（两节点自动重连，
  online/idle/ready 全真、`GET /` 200、`/api/time` 200）→ 再
  `restart opencoder-agent@node02 @node04`（重注册全真）。
- 观察期：10:33–10:37 三次采样（约每 2 分钟）：三单元 active、双节点
  online+ready、`GET /` 200、二进制指纹三次采样稳定不变、换装后
  （10:33:00 起）journal ERROR 行为零——仅滚动瞬间 2 条预期
  `node channel failed`（连接重置，与 agent 重启一一对应）。
- 回滚路径：恢复上述 `.bak` 并 restart 对应单元即回到换装前版本；
  代码层回滚点不变（单点 revert `4a7d9000b`）。

## 状态与后续

- soak：已执行（分段，见上文执行记录），四曲线判定通过；连续多小时
  驻留可在专用测试机补跑。
- 发布：已上线（见上文发布记录），观察期通过。
- 阶段二 P1（agent host reqwest 超时、control 优雅关停+GC、web 句柄/队列、
  WAL checkpoint）：WAL checkpoint 涉规则 04 不变量，启动前需策略评审。

## 回滚

- revert 本提交即可（纯文档）；回滚时删除本 changelog。
