Commit: 2aa44247d199d782881b9ee64921c4c6de2e6199

# streaming 巡检常态化与两集群全绿基线（零代码改动）

## Context

`streaming-inspector` 自 [注册与冒烟](../2026-09-17/streaming-inspector-agent-registered.md) 后演进为 5 分钟定时巡检（`agent-streaming-inspect-5m-*`），工具链定型为 `vstream-cluster-facts`（先采 facts）→ `vstream-ns-ops`（三 probe 快照）→ `vstream-ticket-raise`（per-probe 开单），按 how.md 三步编排（父代理采证据 → 仅 abnormal 时派 explore 子代理并行归因 → 父代理逐 probe 开单）。opencoder 仓库无代码改动，本条沉淀巡检判定语义与首个两集群全绿基线。

## 判定语义（稳定口径）

- **门禁唯一性**：abnormal 判定与开单只看 `vstream-ns-ops` 快照三 probe（`flink_job` / `lag` / `node_capacity`）。facts 工具 stdout 的分段自评不进门禁——本轮两集群自评均带 latency 段 critical（`display_issue_count=2`、`display_contract_valid=false`，图表展示契约检查），但三 probe 实测全 normal，只如实回显不开单。
- **known_gap 不判异常**：jy-hub `sink_buffer_to_ack_ms` 无序列属 meta 登记缺口（generation=source-offset-latest-v1，主 row-stream-load sink 按设计空闲，当前交付走 exact-correction），以 warning 回显，恢复条件登记为「激活 event 或切换 generation 使 gauge 出现非负样本」。
- **开单纪律**：一律 per-probe 且强制 `--probe`，禁止 combined；`deduped` / `already_exists` 视为已处理。
- **先采证后判读**：facts 缺失会让 ns-ops 判 `evidence_gap`，每轮必须先 `--collect`；本轮采集 23 probes 无失败（两集群 `machine-start` 各重试 1 次成功），host_set 与 meta 十台全对齐。

## 基线记录（2026-09-27T08:41Z 轮）

| cluster | 三 probe | 关键值 |
| --- | --- | --- |
| viking-streaming-prod | 全 normal | subtasks 2586/2586、vertices 15/15、TM 60；lag 0.395s；checkpoint 年龄 82.4s；sink 失败(5m)=0 |
| jy-hub | 全 normal | subtasks 1038/1038、vertices 19/19、TM 20；lag 0.129s；checkpoint 年龄 30.5s；sink 失败(5m)=0 |

零开单。快照：`/data00/viking-streaming/agent-ops/namespaces/{viking-streaming-prod,jy-hub}/runtime/reports/2026-09-27T084125Z.json`；audit 落 `runtime/audit/inspections.jsonl`。

## Impact / 兼容

- 巡检从「注册+冒烟」转为常态化运行，后续轮次以本条为基线参照；能力本体演进仍以 `/data00/viking-streaming` 仓库为准，需同步注册池内 tools/skills。
- 稳定能力文档见 [streaming 巡检](../../streaming-inspection/index.md)。

## Related Docs

- [streaming-inspector 注册与全链路冒烟](../2026-09-17/streaming-inspector-agent-registered.md)
- [版本化 Agent 与 NFS](../../../agents/agents/index.md)
