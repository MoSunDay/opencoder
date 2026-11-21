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

## 状态与后续

- 全量 soak：待时间窗批准（预检已通过，随时可启动）。
- 发布（rolling bundle → server 滚动 → 观察期）：待用户决策；四修复均为
  server/节点侧，无客户端兼容前置，先滚 server 即可。
- 阶段二 P1（agent host reqwest 超时、control 优雅关停+GC、web 句柄/队列、
  WAL checkpoint）：WAL checkpoint 涉规则 04 不变量，启动前需策略评审。
- 回滚点不变：单点 revert `4a7d9000b`。

## 回滚

- revert 本提交即可（纯文档）；回滚时删除本 changelog。
