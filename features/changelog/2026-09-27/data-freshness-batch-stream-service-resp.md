Commit: 2460d34d3da46b85ab73990c1a9ae6885fe651b9

# data-freshness 批次执行：stream_service resp 域 16 成员（零代码改动）

## 背景

frozen 批次 `dag-freshness-ea7e95de1ea7e579438903f74ffadc3d955f141d1a2cef0c`（domain=resp，kind=analysis，16 成员，`members_sha256=5a893510…`，repo `163cfe9e826c6d5bb267bd77d03016ef` videocut-aigc/stream_service @ commit `e6d345ad193b446e53520578fba83fdd8985709b`）执行完成：`task.py source` 一次性物化精确源码（归档 sha256 `598e8043…`，133302436 字节，凭据 `source.json`），逐成员独立分析后 `task.py finish` 封存，仅返回 `vta.data-freshness.batch-result-reference.v1` 引用（result.json sha256 `a8f8e56982664484578e9ed87a93c89214b4c5d98c196de50be157e609e85163`，3165062 字节）。opencoder 仓库无代码改动，本条沉淀本次新遇到的契约口径与 resp 域结论基线。所有成员 prior_conclusion=null、base_version=0、无 open_contradictions，16/16 无矛盾。

## 批次契约要点（本次新增口径）

- kitex Go 仓库响应结构在 vendored `kitex_gen`（json tag 即线端字段）；归档 `not_include_paths` 排除 `idl`，无 IDL 可查，生成结构体即最终事实。
- `evidence.entry_method` 可为空字符串：按字段路径 + 信封语义映射入口（`data.data_list` → 公共 Search 接口），并排除同名嵌套字段的调试接口（`FetchDebugSearchData.data_list`）。
- 值级断言（`errmsg=='success'`、`ret=='0'`）在 sug/guess 上是条件性成立：需参数校验通过且召回成功；`logid` 非空/长度>0 是无条件断言（`LogIDDefault` 空值回退 `"-"`）。
- 空列表断言（`suggest_list`/`guess_list`/`data_list` 非空）只能条件性成立：早退分支列表为 null（Search 的 `resp.Data` 仅成功路径构造，错误分支 `data` 为 null），成功路径也可能为空（TCC 配置未命中/空召回/builder 空结果）。

## resp 域结论基线（@ e6d345ad，stream_service）

- 信封：`pkg/respext/respext.go:16-25` Set 统一写 ret/errmsg/systime（`Unix()` 的 `%d` 串）/logid（`ctxvalues.LogIDDefault`）。`ErrSuccess{"0","success"}`、`ErrParam{"1000","invalid parameter"}`（kitex_gen `videocut/mweb/api/api_common_struct.go:281-295`）；`ErrSearch{"11000"}`、`ErrSearchGuess{"11100"}`、`ErrSearchSug{"11200"}`、`ErrSearchShark{"11300"}`（kitex_gen `videocut/dreamina/stream/error.go:29-48`）。
- Sug（entry `cc0601b8…`，POST `/mweb/search/v1/sug` → `handler/search/sug.go:27`）：`SugResponse`（search.go:6073-6080）+ `SugRespData.suggest_list`（:6597）；终态 ret ∈ {0,1000,11200}：req nil:32 / 参数校验:41 → 1000，keyword>50 rune:66 与反转实验:101 → 0（suggest_list 保持 nil），qrec 失败:132 → 11200，成功:160 → 0（仅非空 Query 装配 :153-156）。
- Guess（entry `7b20a914…`，`/mweb/search/v1/guess` → `handler/search/guess.go:21`）：`GuessResponse`（:7717-7724）+ `guess_list`（:8242）；ret ∈ {0,1000}：count∉[0,10]:34 → 1000，反转实验:72 → 0，count==0 走 TCC 配置、否则 qrec 失败降级 TCC（guess.go:79-101；`biz/guess/guess_search.go:386-410` 按 AppID+Entrance 匹配、洗牌、裁剪），终态:106-108 → 0；`ErrSearchGuess`(11100) 已定义但 Guess 全分支未使用。
- Search（entry `32f8c993…`，entry_method 空 → 公共 Search，`handler/search/search.go:27`）：`SearchResponse`（:2501-2508）+ `SearchRespData.data_list`（:4254）；ret ∈ {0,1000,11000,11300}：req nil:32 / 未知 channel:175 / 校验失败:181 → 1000，shark:219 → 11300，Build 其他失败:222 → 11000，成功:232-243（`resp.Data` 仅此处构造，`DataList=buildResponse.SearchItemList`:236）。

## 验证

16 成员独立成稿（conclusion 含非空 fields+assertions，checks 覆盖 response_fields+interface_assertions 且全部 passed，引用 pinned commit file:line），逐位对齐 frozen 顺序（task_id 由解码成员列表回映射生成，非手打）；成员分布 sug 7 / search 5 / guess 4。本次为静态源码核查，未执行 go build/go test；handler 既有测试仅覆盖 channel 分类（search_test/sug_test/stability_test），不覆盖信封字段。result.json 3165062 字节 < 8 MiB，未触发 `batch-failure.v1` 拆批路径。

## Impact / 兼容

无 opencoder 代码影响。为后续 stream_service resp 域（verification 复核、矛盾判定）及其他 kitex Go 仓库 resp 批次提供契约口径与结论基线，减少重复源码追链。沿用既有惯例，不沉淀独立特性页（一次性 frozen 批次执行）。
