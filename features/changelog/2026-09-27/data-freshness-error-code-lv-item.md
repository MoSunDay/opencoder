Commit: 410f44e0f7a09b248331047305cf9150c51415be

# data-freshness 单成员执行：error-code 域分析（零代码改动）

## 背景

frozen 任务 `dag-freshness-f2357abf20bef59f26e0d679c2ff37059cc9a4ab08061373`（domain=error-code，kind=analysis，单成员非批次，base_version=0，prior_conclusion=null，review_brief 空，required_checks=error_definition+reference_mapping，psm=faceu.lv.item）由 VTA freshness 执行完成：repo `262539411c985ecbc2339641453f65e9`（faceu-server/lv-item，thrift Go item 服务）@ commit `00bf1141dcf65e4b58057593e9ad08837cf46dd5`，source_version `1c007a464722adecd297723f56f41fc350ef9ab7b755f25f362285d3a6a15307`，归档 5458 文件，receipt sha256 `3ef0ad0dee65db152dcdbed4ef43d593490f93cae4d52d07c489b8be9b0e473b`。`task.py input→source→finish` 封存，仅返回 `vta.data-freshness.agent-result-reference.v1`（result.json 800638B，sha256 `82f60a077cd57b8f5742f4ab0d2b717e4ade10927d4fbdb6c8b64e2009a51fea`；conclusion content 795334B 含 helper 自动补的 coverage，definitions 4 + references 9）。opencoder 仓库无代码改动，本条沉淀 error-code 域结论基线与本次新口径。

## error-code 域契约要点（本次新增口径）

- conclusion 8KiB 预算是冻结指令的软约束（instructions.md 明文，超限应返回 task error），task.py 校验器实际只查 draft 键集、note/check 字段 2048B、checks 1-100 与 8MiB 传输上限；error-definition 类 25 值枚举全量入 conclusion 时用紧凑行式（`value_name_prefix` + 行数组 [名称后缀, 数值, 含义]）代替逐值对象，8179B 收口。
- thrift Go 仓库枚举有双份定义：仓库内嵌 `idls.go`（item.thrift 原文）与 vendored `thrift_gen/*/ttypes.go`，需逐值对照并脚本化 diff String()/RetCodeFromString 的 case 集合与枚举集合。
- 单成员 analysis 的每条 check 须绑 pinned `file:line` 或真实构建/脚本输出（沿用 cache-refresh-go 条目口径），纯 Action check 不可用。

## error-code 结论基线（@ 00bf1141，供后续 verification/批次复用）

- 唯一自有码族 `lv_item.RetCode`（thrift enum int64，承载于 `base.BaseResp.StatusCode` i32）：`idls.go:39-70` 权威、`ttypes.go:24-49` 镜像，25 值 = OK=0、EBEGIN=41000000、ESERVICE=41000001、EPARAM=41000002、41010001-41010014 端点族（submit_video_item/batch_get_item/delete/offline/get_user_item_list/get_status_item_list/item_not_exist/online/add_usage_amount/ban/review_callback/like/unlike/get_like_item_list）、41020001 batch_get_user、41020015 item_status_changed、41020016 create_aweme_share_style、41020017 get_hot_rank、41020018 batch_get_item_base、41020019 add_usage_amount_exceed_limit、EEND=41999999（段上限 <2^31，int32 无损）。
- String()（ttypes.go:52-106）25 case 全覆盖、未知回退 `"<UNSET>"`；RetCodeFromString（108-162）25 名全覆盖、未知回退 RetCode(0)+error 且服务代码零调用；22/25 值被服务代码引用，EBEGIN/ESERVICE/EEND 为零产生分支哨兵。
- 唯一公共出口 `base.BaseResp`（idls.go:20-32，StatusMessage 默认 ""、StatusCode 默认 0；NewBaseResp 零值）；唯一写手 `model/dtm/base_response.go:15-17` SetCode，controller 73 个调用点；`MakeBaseResp`（8-13）零调用方死代码。17 个端点 Go error 恒 nil，每端点恰一处 defer `KiteAPIMetricsEmit` 导出 `rsp.statusCode`/`rsp.statusMessage`（metrics.go:72-84，api 名 17 个，get_status_item_list→get_status_item、get_user_item_list→get_user_item 为简写）。
- EPARAM 20 产生点（17 处 check* + 3 处 GetInternalItemStatus 失败，StatusMessage=err.Error() 原文）；biz.GetItems err → EITEM_NOT_EXIST ×8，空结果按端点分派（like/unlike 用 EITEM_NOT_EXIST，其余 6 端点用各自码，消息均 "get item nil err"）；AddUsageAmount 为唯一成功路径显式写码（controller/add_usage_amount.go:38 `SetCode(retCode, retCode.String())`，biz/counter.go:20-40：限流→EXCEED_LIMIT、PostCount 失败→EADD_USAGE_AMOUNT、成功→OK）；EITEM_STATUS_CHANGED=41020015 为 ReviewCallback 的预期性非错误返回（IDL 注释：修改状态前，状态已经发生变更）。
- 下游 BaseResp.StatusCode 消费：rpc 层 9 处检查（aweme_open/lv_account/review_queue 转匿名错误文本、smart_player 通用 "invalid response statusCode"、counter×2/item_info×2/user 仅记日志），biz 层 11 处复检格式化 `code=%d, msg=%s`（like.go:105 仅日志）后折入端点 RetCode；下游数字码从不映射为 lv_item.RetCode；rpc/favorite、rpc/item_list、rpc/video、rpc/video_transcoder 不检查下游 StatusCode。

## 验证

exact copy（非工作树）内编译运行 enumcheck（`GOFLAGS=-mod=vendor go run`）断言 25 值与 String()，输出 `checked=25 fail=false` 退出码 0；vendored sonic/pid 汇编与本地 Go 1.26.5 不兼容，`./controller ./biz ./rpc` 无法整模块构建，`./model/dtm`、`./constant`、thrift_gen 可编译（与 cache-refresh-go "整模块可构建"口径相反，属 vendored asm 阻塞型）。17 个 controller 全量 1426 行通读（非 grep 片段），SetCode 调用清单 75 行 = 73 调用点 + 2 定义；全部 checks 引用 pinned `file:line` @00bf1141 或上述真实运行输出。

## Impact / 兼容

无 opencoder 代码影响。为该 repo 的 error-code 后续 verification 轮（platform 下发 prior conclusion 与其 `conclusion_sha256`）与同构 thrift/vendored 仓库分析提供事实基线、8KiB 压缩口径与 vendored asm 构建口径；沿用既有约定，暂不沉淀独立特性页（一次性 frozen 执行，非常态化巡检）。
