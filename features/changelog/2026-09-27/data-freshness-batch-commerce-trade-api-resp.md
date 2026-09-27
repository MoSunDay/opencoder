Commit: 6ab6ec63595b45b7440f047d768fff7108a6ab04

# data-freshness 批次执行：commerce_trade_api resp 域 16 成员（零代码改动）

## 背景

frozen 批次 `dag-freshness-264b4e8c56d0c09c1689d2e95875a2f5e1e0a70fafd4a8bc`（domain=resp，kind=analysis，16 成员，`members_sha256=3573260e…`，repo `0622a1146bfc8061050a15af8e451eea` @ commit `ad8ebdedc2b34bb5bdab3cda0984a7b2b8d1db59`）由 runtime skill `frozen-data-freshness`（viking-data-freshness-agent）执行完成：`task.py source` 一次性物化精确源码（归档 sha256 `f796ad5e…`，凭据 `source.json`），逐成员独立分析后 `task.py finish` 封存，仅返回 `vta.data-freshness.batch-result-reference.v1` 引用（result.json sha256 `7c5b79529a492014e750251c1b17b88b945dcd82d0bfefdd5e5cf00bf7117771`，1908595 字节）。opencoder 仓库无代码改动，本条沉淀批次执行契约要点与 resp 域结论基线。所有成员 prior_conclusion=null、base_version=0、无 open_contradictions，16/16 无矛盾。

## 批次契约要点（稳定口径）

- 单成员 draft 恰为 `{conclusion, finding, checks, note}`；resp 域 conclusion 必须含非空 dict 列表 `fields`+`assertions`；checks 为 1–100 个 `{claim, capability, passed, reference}`（字符串 ≤2048B，reference 指向 pinned commit 的真实源码位置或真实测试结果）。
- 批次 draft 恰为 `{"members":[{task_id,draft}|{task_id,error},...]}` 且必须**逐位对齐 frozen 顺序**，漏项/多余/乱序直接失败。实操坑：不要手打 task_id——先按 `(entry_method, field_name)` 生成各成员草稿，再回映射到 `input.json` 的成员列表产出最终文件，可同时保证 task_id 精确与顺序 frozen。
- 超过 8 MiB 产出结构化 `batch-failure.v1`（`result_too_large`），原样返回引用由 VTA 拆批重试；不得截断或跳过成员。
- 封存命令：`task.py finish --task task.json --source <workdir>/source --receipt <workdir>/source.json --draft draft.json`；产物不回显，只返回打印的小引用。
- Go vendor 仓库的构建验证口径：`CGO_ENABLED=0 GOFLAGS=-mod=vendor GOPROXY=off` 只构建被检字段所在包路径；vendored cgo 包（如缺 `rsa_sign.h` 的 lv_pack_common/utils、CGO 下才编译的 gozstd）不在目标包 import 路径上时属非阻塞。

## resp 域结论基线（@ ad8ebded，供后续批次复用）

- 信封 `dtm.CommonAPIResponse`（`biz/models/dtm/ret.go:158`）：`systime` 恒为 UnixNano/毫秒的 `%d` 十进制数字串；`log_id` 取 ctxvalues.LogID、缺省回退 `"-"`——两者在含错误响应的任意响应上恒存在非空。
- iap/purchase 的 `cycle_unit/currency/total_amount/real_amount/valid_time/payment_type/subscribe_cycle/subscribe_type` 全部为下游订阅/交易服务的纯透传（`packSubscribeIapPurchaseResponse`），仓库无常量赋值点；值级断言（如 MONTH/CNY/auto/>0）是端到端断言，源码只能证明存在性。
- purchase/make_order 的 `goods_id_str`/`goods_type` 是**请求回显**（服务端仅校验不改写，`aigc_oneoff` 经 `utils.IsValidGoodsV2` 合法值校验）；make_unauto_order 的 `order_id` 仅存在于 `data`（信封无顶层 order_id）。
- v2 sign_and_pay 的 `subscribe_id` 非空依赖下游签约成功路径；v3 init_trade 的 `cj_aca_result.sdk_params` 为两层 optional+omitempty，仅 trade_ver=v2 且 `CJACAResultV2 != nil` 时出现（v1 链路整键缺失，非 null）。

## 验证

被检目标包（handler/service/common/old_logic/entity/model/models/utils/constants）在归档完整源码副本上 `go build` 全部退出码 0；16 个成员各自独立成稿（无跨目标复用 checks/事实），封存引用经 helper 绑定 frozen 批次与完整覆盖清单。

## Impact / 兼容

无 opencoder 代码影响。为后续 data-freshness 批次（其他 repo/domain 或 resp 域成员复核）提供契约口径与 resp 域既有事实基线，减少重复源码追链。暂未沉淀独立特性页（一次性批次执行，非常态化巡检）。
