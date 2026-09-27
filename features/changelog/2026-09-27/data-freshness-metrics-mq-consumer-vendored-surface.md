Commit: 410f44e0f7a09b248331047305cf9150c51415be

# data-freshness 单成员执行：mq_consumer metrics 域（零代码改动）

## 背景

frozen 任务 `dag-freshness-2cb12e322d0c973583e7bf6736104caa9497af7871fcb128`（domain=metrics，kind=analysis，单成员，base_version=0，prior_conclusion=null，review_brief null；repo `pippit.business.mq_consumer`，repository_id `1a36b0419f2d31fbdd07aa5e7e3d8c75`，target_id `dec1d56d9043c65f3a5834f37ae1c2ae601e1ff1` @ commit `17e658a41f59edeee5fd10c1a60b96e59066418b`，source `ec85918d…`，归档 sha256 `1b12fc15…`，receipt sha256 `ede1a80f…`，10444 文件其中 10394 vendor）由 runtime skill `frozen-data-freshness` 执行完成：`task.py source` 物化源码树，`task.py finish` 封存并返回 `vta.data-freshness.agent-result-reference.v1`（result.json 1605713 字节，sha256 `c9378ed4d6d271d6263aa5e8e76bca779b32d198bd6b6523b4bc72af6aa9706b`）。opencoder 仓库无代码改动，本条沉淀该 repo 的 metrics 域结论基线，并修正[同日 metrics 条目](data-freshness-metrics-domain-analysis.md)的尺寸口径误记。

## 契约要点（对前条的修正）

- conclusion 尺寸以 instructions.md 为准：本任务 Result contract 写明 "Conclusion JSON must fit 8 KiB. If complete facts cannot fit, return a task error"，初稿 13255B 超限，压缩至 8180B 后封存；task.py 自身仍只有 result 整体 8 MiB（MAX_BODY）与字段级 2048B/note 2048B/finding 4096B 限制。单成员 finish 口径与[前条](data-freshness-metrics-domain-analysis.md)一致：draft 恰为 `{conclusion, finding, checks, note}`，`task.py finish` 内部完成封存并打印引用，无需单独 publish。

## metrics 域结论基线（@ 17e658a4）

- 首方 25 个 go 文件（biz/mq/infra/clients/common/config/main.go/handler.go + kitex_gen + conf/config）零指标定义/注册/产出，可观测仅 logs.CtxInfo/CtxError（mq/consumer.go:58-66）；首方只 import rocketmq 的 pkg/config、pkg/consumer、pkg/pb、pkg/types（无 producer 用法）。
- 运行时指标面全部来自 vendored `code.byted.org/rocketmq/rocketmq-go-proxy` v1.6.22：`initMetrics()`（pkg/metrics/metrics.go:184-240）经 `metricsv4.NewClient("rocketmq.client",SetDiscardInvalidTag[,SetGlobalTags])` 在 sync.Once 内注册 43 个 v4 指标，由 `consumer.NewConsumer`（consumer.go:586）与 `NewConsumerMetrics`（metrics.go:346；consumer.go:672、multi_conn_consumer.go:154）触发——本服务 5 个 consumer（conf/server_conf.yaml，cluster web_common5）各触发一次；导出走 gopkg/metrics v4 默认 SDK → 本地 metrics agent（v4/sender.go:11-17）。legacy v2 client（metrics.go:88）仅 mqmesh interceptor 使用。
- 指标分组：consumer.success/error.{throughput,latency.us}、failure.drop.*、consume.slow.throughput（阈值 ConsumeCheckTimeoutMillis 默认 30000ms）、b2c/s2c.ms、recv.*（含 failed.crc）、sendCmd.*、reconnect/rebalance/not_in_consul、ratelimit/filter/dedup.latency/buffer.*、version（Handshake 心跳 rpc/client.go:247）；producer.* 15 个注册但本进程不可达（CircuitBreaker 由 ProducerConfig 构造 lb/circuit_breaker.go:26）；7 个死常量（metrics.go:44-58，pull.* 标 Deprecated）未注册未产出。
- 一致性要点：活路径为 orderly 单条（mq/consumer.go:63 Orderly=true、仅 RegisterHandler，batch/unordered emit 点全部不可达）；EmitConsume 每 retry attempt 成败对偶（consumer.go:509,512）；`consumer.recv.failed.*` 仅注册不产出（唯一调用点 inner_consumer.go:370 恒 errored=false）；`consumer.dedup.latency` 无去重器也逐条产出 0（doDedupPreCheck nil 返回 0，consumer.go:450-454）；ratelimit 不可达（EnableRateLimit 默认 false，config/consumer_config.go:308）；mqmesh 三指标 env 门控 `MESH_AGENT_SIDECAR_IS_ENABLED_INF_MQMESH=1` 且 !DisableMQMesh；kitex 侧由生成代码的 byted.ServerSuite 产出 kitex.request/process.throughput、service.thrift.<method>.calledby.error.throughput（KX/internal/metrics/tracer.go:24-27）与 apache_codec_rate（PSM kitex.apache_codec.warning）。
- tag 口径：T1（metrics.go:152）= cluster,topic,consumer_group,client_type,instance_id,version,language,broker_name,queue_id（instance_id 仅 ShouldUseVolcano）；T2 = T1 − broker_name,queue_id；T3 = T2+packet；T4（producer）= topic,cluster,code；latency 后缀 .us 微秒、.ms 毫秒、裸 .latency 微秒。

## 验证

9 checks（4 metric_definition + 5 instrumentation_consistency）全部 passed，引用 pinned commit 的真实 file:line（含逐常量出现次数、逐 Emit* 调用点扫描）；conclusion 8180B ≤ 8KiB，finding/note/check 字段均在限内；封存引用经 task.py 绑定 frozen 任务与已验证源码树。

## Impact / 兼容

无 opencoder 代码影响。为该 repo 后续 verification 轮（需引用 `conclusion_sha256`）与同类 "零首方指标 + vendor 指标面" Go 服务分析提供事实基线；并修正同日 metrics 条目的 conclusion 尺寸口径，避免后续执行按旧口径超限。沿用既有约定，暂不沉淀独立特性页（一次性 frozen 任务执行，非常态化能力）。
