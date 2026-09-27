Commit: 6ab6ec63595b45b7440f047d768fff7108a6ab04

# streaming 巡检

对 viking-streaming-prod 与 jy-hub 两集群的定时只读巡检，由平台注册自定义 agent `streaming-inspector` 承载，当前以 5 分钟节奏常态运行（`agent-streaming-inspect-5m-*`）。巡检只读采证与判读，不触碰集群运行时。

## 编排（how.md 三步）

1. 父代理采证据：`vstream-cluster-facts --collect` 先落 facts（缺失会让 ns-ops 判 `evidence_gap`，必须先采集），再 `vstream-ns-ops` 产出快照。
2. 仅存在 abnormal probe 时，按 probe 派 explore 子代理并行归因；全 normal 直接简报，不派子代理。
3. 父代理逐 abnormal probe 开单：`vstream-ticket-raise --report <快照> --probe <probe>`，一律 per-probe 并强制 `--probe`（禁止 combined）；`deduped` / `already_exists` 视为已处理。

## 判定口径

- `vstream-ns-ops` 三 probe（`flink_job` / `lag` / `node_capacity`）是唯一 abnormal 判定与开单依据，以快照 `status` 为准。
- facts 工具 stdout 的分段自评（如 latency 展示契约检查）不在三 probe 门禁内，只如实回显，不据此开单；自评分段会随集群状态新增且可能长期持续（2026-09-27 轮 prod 另见 `flink` 段 critical，对应 facts `flink.runtime_controls` effective-state readback unavailable、自 2026-08-26 起（2026-09-27T11:21Z、12:55Z、13:26Z、13:35Z、13:41Z、13:51Z、14:16Z 与 14:26Z 轮实测 delivery_reason=「Flink effective-state readback failed」，回读走 127.0.0.1:19081-19083 本地端口不可达，作业本体经 REST 10.229.200.20:8081 与 Prometheus 双源确认健康），同块 applied 的 jy-hub `flink` 段为 pass；prod `starrocks` 段 warning 已复现（2026-09-27 至少十五轮，含 08:41Z、10:16Z、10:20Z、10:46Z、11:16Z、11:21Z、11:30Z、12:55Z、13:26Z、13:35Z、13:41Z、13:51Z、14:10Z、14:16Z 与 14:26Z 轮（11:16Z、11:21Z、11:30Z、12:55Z、13:26Z、13:35Z、13:41Z、13:51Z 与 14:26Z 轮均三 probe 全 normal、零开单，仅回显）；10:20Z 轮实测 check 为 `starrocks.be_exporter_observation`，expected coverage enabled、actual null）、成因未核），处理方式不变。
- facts probe 级重试不判失败：首试报错（如 `starrocks` 首试 `RuntimeError: BGO probe exited 1`）、重试成功时记入 `collection.retried_probes` 并使 collection 段自评 warning，实测值取重试成功样本；不属三 probe 门禁、不阻断本轮（2026-09-27T11:51Z 轮两集群 `starrocks` 同现，jy-hub starrocks 段自评 pass、prod 段 warning 延续既有未核口径）；重试也会落在 machine-end 端点探针（13:35Z 轮两集群各一次：prod 10.229.192.44、jy-hub 10.10.114.174，重试成功、host_set 十台仍齐全）。
- `vstream-cluster-facts --collect` 在分段自评非 normal 时以非零码退出，stdout 为信封 JSON（观察到带与不带 `[error]` 前缀两种形态，如 10:51Z 轮两集群 latency 段 critical 时为不带前缀的纯信封 JSON），但 facts 文件仍正常落盘；非零退出不等于采证失败，以 facts 文件存在且 `collection` 段 checks 全 pass 为准继续本轮（`retried_probes` check 为 warning 不算采证失败，如 10:46Z 轮 jy-hub `starrocks` 首采 `BGO probe exited 1`、重试成功后 facts 完整），勿据此中断或开单。自评层自带 `fact_age_seconds` 检查（同为 90s 窗口）且按评估时刻计龄：对已落盘 facts 做 `--facts` 复评超过窗口时会把相关段抬级（2026-09-27T13:20Z 轮 prod 复评 102s 龄 facts 实测 `starrocks` 段由当轮 fresh 采集的 `warning` 抬成 `critical`、`flink`/`latency` 段混入 fact_age 项并退出码 2），复评非零只反映证据龄，不代表当轮门禁或既有自评口径回退。
- meta 已登记的 known_gap（如 jy-hub `sink_buffer_to_ack_ms` 在 source-offset-latest-v1 下主 sink 按设计空闲、序列缺失）按 warning 回显，不判 abnormal、不开单；恢复条件随登记条目删除。
- `node_capacity` 的 `host[*].fact_age` 是证据新鲜度门禁（meta `max_fact_age_seconds=90`，按快照 `observed_at` 减 facts `captured_at` 计）：facts 采集完成到快照生成之间的编排间隔超过上限时，全部主机以同一 check 整批判 `evidence_gap` abnormal——属 fail-closed 证据缺失，不代表真实容量超标；归因确认最近一次实测均在阈值内后不新增容量处置，工单按 deduped 在办单沿用。该 signature 间歇复现（2026-09-27 09:33Z 两集群整批、10:32Z 仅 prod，间隔 100–112s 边际超限；采集段时长均远低于 deadline，缺口在采集完成到快照生成之间）。
- `node_capacity` 超标有两类成因，均在 facts 里可复核：`evidence_gap`（见上）与真实超标（`load_per_core` 实测超 meta `thresholds.max_cpu_load_per_core`，可由 `machines[].load_1m / cpu_cores` 复算，2026-09-27T10:26Z 轮 prod host 10.229.192.44 实测 1.541875 > 1.5，同轮数据面 probe 全 normal、未传导；11:55Z 轮复现于 10.229.200.19（1.5347，即 10:26Z 轮次高机，上轮超标机 .44 回落 0.94/核，超标点在 32 核 flink 组内轮动；12:25Z 轮第三次超标仍落同一台 10.229.200.19（1.6384，+9.2%，连续两轮同机——轮动不保证每轮换机，幅度可放大至 ~10%）；12:25Z 后 12:31Z 轮回落 normal、12:36Z 轮第四次超标仍落 .19（1.6322，与 12:25Z 基本持平）——同机为「超标→回落→再超标」的间歇尖峰而非持续过载，四轮均 dedupe 同一在办单；13:35Z 轮 prod 回落 normal，最高 1.3088/核落 10.229.192.44（即 10:26Z 超标机）；13:41Z 轮延续 normal，组内最高回落至 .19 1.0675/核（与 .44 1.0625 基本持平，未再破线）；13:51Z 轮延续 normal，组内最高 1.4397/核落 10.229.192.44（即 10:26Z 超标机，仍低于 1.5 门禁）；14:02Z 轮延续 normal，最高点首次落 10.229.192.25 1.1425/核（记录内第 4 台不同主机），checkpoint 年龄 145.9s→124.7s 与最高负载同向下行，单机因果 watch 继续弱化；14:10Z 轮延续 normal，最高点回到已见机 10.229.192.44 1.2537/核（详见 [node_capacity 首次真实超标](../changelog/2026-09-27/streaming-patrol-node-capacity-real-breach.md)）；14:16Z 轮延续 normal，最高点回到三轮超标机 10.229.200.19 1.4831/核（门禁 98.9%，未破线，最高点不同主机数仍为 4）而 checkpoint 年龄 83.4s→30.3s 继续下行，负载回升与 checkpoint 下行的背离延续；14:26Z 轮延续 normal，组内最高回落至 10.229.200.20 0.864375/核（normal 追踪段最低），checkpoint 年龄 30.3s→128.0s 回升（flink_job probe 仍 normal），负载回落而 checkpoint 年龄回升，两信号反向变动延续、方向与上一轮翻转。真实超标同样走 per-probe 开单；幂等键按 cluster+probe 计数、不区分成因，因此与 evidence_gap 轮 dedupe 到同一在办单，不新增处置单（见 [node_capacity 首次真实超标](../changelog/2026-09-27/streaming-patrol-node-capacity-real-breach.md)）。

## 证据链

- facts：`/tmp/streaming-inspection-facts-<cluster>.json`（单轮临时证据；固定路径会被下一轮 `--collect` 覆盖，须当轮即时采信，持久证据只有快照与 audit）。`--collect` 退出码 `0`=全部通过、`2`=分段自评存在 warning/critical、`1`=registry/事实包无效（契约见 `/data00/viking-streaming/skills/inspect-streaming-cluster/`）；facts 落盘且 `collection` 段 checks 全 pass 即有效证据，退出码 2 不构成 evidence_gap。
- 快照与 audit：`/data00/viking-streaming/agent-ops/namespaces/<cluster>/runtime/reports/` 与 `runtime/audit/inspections.jsonl`；编排 tee 的本地副本 `/tmp/nsops-<cluster>-<ROUND>.json`（单轮临时，与 facts 同类）。
- 工单：per-probe 一行回执；无 abnormal 则零开单。

## 边界

- 探测项、门禁阈值与对账口径以 `/data00/viking-streaming` 仓库（`agent-ops/`、`skills/inspect-streaming-cluster/`）为准；本仓库只承载注册、调度与运行。
- 凭据不进注册池（池经 NFS 只读导出）；viking token 由包装脚本运行时注入。

## 相关

- [版本化 Agent 与 NFS](../../agents/agents/index.md) — 注册与共享池机制。
- [注册与冒烟记录](../changelog/2026-09-17/streaming-inspector-agent-registered.md)。
- [巡检常态化与全绿基线](../changelog/2026-09-27/streaming-patrol-two-cluster-green.md)。
- [fact_age 证据缺口首轮](../changelog/2026-09-27/streaming-patrol-fact-age-evidence-gap.md)。
- [node_capacity 首次真实超标](../changelog/2026-09-27/streaming-patrol-node-capacity-real-breach.md)。
