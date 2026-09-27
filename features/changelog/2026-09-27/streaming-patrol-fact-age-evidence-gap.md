Commit: 6ab6ec63595b45b7440f047d768fff7108a6ab04

# streaming 巡检首个 fact_age 证据缺口轮（两集群 node_capacity 判 abnormal）

## Context

2026-09-27T09:33Z 轮（编排 ROUND 20260927T093030Z）常态巡检中，两集群 `node_capacity` 整批判 abnormal：快照 `observed_at=09:33:07Z` 与 facts `captured_at`（prod 09:31:17Z / jy-hub 09:31:15Z）间隔 110–112s，超过 meta `max_fact_age_seconds=90`，十台主机以同一 check `host[*].fact_age` 触发；`flink_job`、`lag` 全 normal（jy-hub lag 含已登记 known_gap 的 warning 回显）。facts 采集本身两集群均成功（23 probe 全 success，时长 45.5s / 43.8s < deadline 120s）。与 [全绿基线轮](streaming-patrol-two-cluster-green.md)（同日 08:41Z）相比，这是常态运行后首个 abnormal 轮。快照：`/data00/viking-streaming/agent-ops/namespaces/{viking-streaming-prod,jy-hub}/runtime/reports/2026-09-27T093307Z.json`。

## 判定与处置

- explore 子代理并行归因，两集群结论一致：fail-closed 证据缺失，非真实容量超标——最近一次实测均在阈值内（prod：load_per_core 最大 1.08 < 1.5、可用内存最小约 27GiB > 16GiB、磁盘最高 57.99% < 84%；jy-hub：load_per_core 最大 0.369 < 1.0、可用内存最小 21.8GB、磁盘最高 5.33%；oom_kill_delta 均为 0）。
- 缺口仅影响 `node_capacity`：`flink_job` / `lag` 指标走 Prometheus 实时查询（`http://10.43.123.186:9090/prometheus/api/v1/query`，`errors={}`），不消费 machines facts。
- 开单：`vstream-ticket-raise` 两集群均 `deduped`（在办单 `fdd8b554-7dc8-4444-b3e1-0497e33f09a6` / `9782b1df-f87a-48fb-b94f-32ad1ff3f637`，幂等键 `…-node_capacity-2` / `…-3`），按口径视为已处理，零新增处置。

## Impact / 兼容

- 判定口径补入 `fact_age` 证据缺口语义（见 [streaming 巡检](../../streaming-inspection/index.md)）：`evidence_gap` ≠ 真实超标，归因确认后不开容量处置单。
- 编排间隔（facts 采集完成 → 快照生成）是该 signature 的直接变量：间隔持续 >90s 时 `node_capacity` 会逐轮以同一 signature 复现；修复方向是压缩该间隔，而非容量处置。
- 待验证（因果候选，字段可证但未复现确认）：快照 `observed_at` 疑晚于探针实际执行约 117s（由 checkpoint 时间戳推算），若成立则超限属快照组装/写盘滞后。同日 09:57Z 轮未复现：facts `collection.completed_at` → 快照 `observed_at` 间隔回落到 prod 55s / jy-hub 5s（<90s），两集群 `node_capacity` 恢复 normal，说明该 signature 为间歇性而非持续性；机制归因仍待后续复现轮确认。
- 复现确认（2026-09-27T10:32:56Z 轮，编排 ROUND 20260927T103030Z）：prod 单集群再次整批 `host[*].fact_age` abnormal，间隔 100s 边际超限（facts `machines[].captured_at=10:31:16Z` 十台同秒 → 快照 `observed_at=10:32:56Z`，+10s）；jy-hub 同轮未复现。采集段本身健康（`collection.duration_seconds=45.499` < deadline 120s、无 failed/retried probes），缺口落在采集完成到快照生成之间（Prometheus 查询+评估+落盘）且该段无对应时间预算——结构性归因候选维持，仍未复现到代码级确认。归因子代理复核最近实测全部在阈值内（load_per_core 最高 0.92 < 1.5、可用内存最低约 27GiB > 16GiB、磁盘最高 57.99% < 84%、oom_kill_delta 全 0），工单 `deduped`（在办单 `fdd8b554-7dc8-4444-b3e1-0497e33f09a6`，幂等键 `…-node_capacity-2`）。快照：`…/viking-streaming-prod/runtime/reports/2026-09-27T103256Z.json`。

## Related Docs

- [streaming 巡检（稳定口径）](../../streaming-inspection/index.md)
- [巡检常态化与两集群全绿基线](streaming-patrol-two-cluster-green.md)
- [streaming-inspector 注册与冒烟](../2026-09-17/streaming-inspector-agent-registered.md)
