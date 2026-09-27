Commit: aa3199f5369c805b681f22d6265b8a8242599684

# data-freshness 单成员执行：metrics 域分析（零代码改动）

## 背景

frozen 任务 `dag-freshness-189b0989e0db43445e594c93eeb487f98a502235f0de7d2b`（domain=metrics，kind=analysis，单成员非批次，base_version=0，prior_conclusion=null，无 open contradictions；repo `093d963be98eb93c0a52477692ddd03d` @ commit `bf57215c8afb2f2c966b5befcedc204070a75c9f`，source_version `2e0684ef…`，归档 sha256 `004df5a5…`，receipt sha256 `1c3312ca…`）由 runtime skill `frozen-data-freshness`（viking-data-freshness-agent）执行完成：`task.py source` 物化精确源码（672 文件，树保留供最终哈希校验），`task.py finish` 封存，仅返回 `vta.data-freshness.agent-result-reference.v1`（result.json 104514 字节，sha256 `07bdc36b645f31826cd292d505aadca471397aa2152fdd5b4ae022e6e635beb5`）。opencoder 仓库无代码改动，本条沉淀单成员执行契约要点与 metrics 域结论基线。

## 单成员契约要点（对照批次版差异）

- 单成员 draft 恰为 `{conclusion, finding, checks, note}`，直接传 `task.py finish --task task.json --source <workdir>/source --receipt <workdir>/source.json --draft draft.json`；**没有**批次版的 members 数组与逐位对齐问题。
- metrics 域 conclusion 必须含非空 dict 列表 `definitions`+`instrumentation`（helper 自动补 `coverage`，勿自带）；本次实测 34 definitions + 13 instrumentation + 20 checks（conclusion 14022B、result 23957B）。
- 尺寸口径：task.py 层 conclusion 确无独立上限（仅 result 整体 8 MiB MAX_BODY），但 instructions.md 另有硬限——本任务 instructions.md:61 即写明 "Conclusion JSON must fit 8 KiB"（超限须压缩或返回 task error）；本条原记 "无 8KiB 上限" 系只核 task.py、漏看 instructions.md，后续 [mq_consumer metrics 条目](data-freshness-metrics-mq-consumer-vendored-surface.md) 实测初稿 13255B 压至 8180B 后封存，实际以各任务 instructions.md 为准；checks 1–100 个、`claim/capability/reference` 字符串 ≤2048B，note ≤2048B。analysis 类 `conclusion_sha256` 恒 null，finding 仍为 `{contradicts, point, summary}`。
- reference 必须指向 pinned commit 的真实 file:line 或真实测试结果；纯动作性 check 会被视为无效。

## metrics 域结论基线（@ bf57215c，SRE 监控 Python monorepo，供后续复核复用）

- `metrics/psm_method.py:149`：`avg:store:bytedtrace.sdk.span.client.rate{_method=*,_to_method=*,_to_service=*}{_psm={service}}`，30s 窗口（end=now-30），服务清单来自 `cmdb.tce_resource_usage` 6h；retry retries=5 backoff 3*2^i；→ `monitor.psm_method`（columns timestamp,service,method,to_service,to_method,qps）。
- `metrics/tlb_domain.py`：tenant `sys.tlb` 的 `tlb.http.request` + `count.rate`，先批量 `service=literal_or(...)` 再按 (service,cluster,domain) `route=*` 下钻；`REGION_RESOURCE_FILTERS` cn=`("cn","sinf")`、sg/us=`("i18n","sinf18n")`；retry retries=30 cap 30s，全局 TIMEOUT → `sys.exit(1)`；→ `monitor.tlb_domain`。**`main` 中 `vregion=="TTP-US"` 分支（:384）不可达**（region_api_map 仅 cn/sg/us 五个 vregion，import 已验证）；真实 US-TTP 等价实现在 `us-ttp-metrics/us_ttp_tlb.py`（grafana-i18n proxy/3014 → `tlb_metrics_data`）。
- `grafana-resource`（tce/tlb/api_code/brief_result 族）：`bytedtrace.sdk.span.server.latency.us` p90 `weighted_avg(pct90,counter)`、`server.rate` qps、`runtime.go.goroutine.num`（tenant apm.runtime）、dc 过滤 `lf|lq|hl`；api_code/brief_result 的 metric 是占位符 `"."`，在 main.py 按 psm/vregion 解析（`{psm}.traffic.rate` vs `{psm}.call.throughput`；CN account 51 goapi vs i18n 133 goapi_global）；上传 `_upload/_upload_v2` → `tce/tlb/api_code_metrics_data`，`Status!=Success` 即 `sys.exit(-1)`。
- 其余生产者：`entrypoint`（server.rate ±`_http_route`、capcut 网关吞吐 → `entrypoint_usage`）、`metrics-fe` JSON 配置（容器 p99 阈值 0.7/0.73、abase error_code 阈值 100、bytequota metric_type_map、dreamina turnover）、`sinf-monitor`（VCM_Redis/VCM_MongoDB → `sinf_metrics`）、`byte-data-sync(-2)`（abase/mysql/hdfs → `byte_*_data`，hdfs helper 拼写 `_uplaod_hdfs_data` 属既有事实）、巡检类统一 POST `cc-dockerhub.byted.org:15555/upload`（sre_inspection_* / sre_alert）、`mid-platform` YAML 指标配置（success_rate → `bytedtrace_sdk_client_metrics`，DB 源可切换）+ ETL、`argos-rule-sync`（success-ratio 规则，total≥3750 门限）与 `argos-server-core`（→ `cmdb.server_core_pathway_psm_v2`）。
- DDL：`metrics/sql.sql` 仅定义 `entrypoint_usage` 与 `server_core_pathway_psm_v2`；其余 `monitor.*`/`*_metrics_data`/`byte_*_data` 等表在该归档内无建表语句（仓外供给）。

## 验证

`metrics/test_tlb_service_config.py` 3 个单测在隔离副本（/tmp，原树零改动）`pytest` 全过（`3 passed`）；import 检查确认 region_api_map 键集、15 个显式 TLB 服务、TTP-US 缺席与 REGION_RESOURCE_FILTERS 取值；全部 checks 引用 pinned commit 的 file:line 或上述真实测试输出。封存引用经 helper 绑定 frozen 任务与完整覆盖清单（expected_files=672）。

## Impact / 兼容

无 opencoder 代码影响。为后续 metrics 域 data-freshness 任务（verification 复核、矛盾判定）提供既有事实基线，减少重复源码追链；单成员/批次两套 finish 口径差异已记录。暂不沉淀独立特性页（一次性 frozen 任务执行，非常态化能力）。
