Commit: aa3199f5369c805b681f22d6265b8a8242599684

# data-freshness 单成员执行：error-code 域分析（零代码改动）

## 背景

frozen 任务 `dag-freshness-7bf949ebc8bb4e14e49449ca3442770ead605653812ed7ab`（domain=error-code，kind=analysis，单成员非批次，base_version=0，prior_conclusion=null，无 open contradictions）由 VTA freshness 执行完成：repo `0cd2304402097dd3d35a76a6af216f4f`（faceu/faceu_server/commerce_cache_refresh，Go binlog 缓存刷新 ByteFaaS）@ commit `984f356b51ea11d95de1a2b24a5d8e7d476cd052`，source_version `69a81953fe19aae037b73ade07dec86ed656efd998754f2494cd4beed81f936d`，归档 2333 文件 sha256 `60484e9334b1f442696240c34837b6fb636ebeae0b6e5aac056ebc731b515e0a`（4753235B），receipt sha256 `0f9ab1a5527576e0eac2b3590db9858ebfb6f388c42dc56f0c469ddd57b4b7f6`。`task.py input→source→finish` 封存，仅返回 `vta.data-freshness.agent-result-reference.v1`（result.json 339594B，sha256 `6b7b4f6d7846ed62b4e9f372228d84538640f9e98b731447ba8138097be94337`）。opencoder 仓库无代码改动，本条沉淀 error-code 域契约口径与该 repo 结论基线。

## error-code 域契约要点（对照 metrics/resp 域）

- error-code 域 conclusion 必须含非空 dict 列表 `definitions`+`references`（metrics 是 definitions+instrumentation，resp 是 fields+assertions）；helper 自动补 `coverage`，勿自带。
- 该 repo 无错误码注册表：应用层 11 个非 vendor Go 文件、0 个 `_test.go`、`grep -iE 'errcode|error_code'` 0 命中；错误面 = 8 处构造点 + 框架公共映射。纯 Action check 不可用，每条 check 须绑 pinned file:line 或真实构建结果。

## error-code 结论基线（@ 984f356b，供后续 verification/批次复用）

- 构造点（三 error 包）：`collection.go:20,25`（stdlib `errors.New(fmt.Sprintf("unknown type:%T",…))`，死代码，无调用方且表分派无 collection case）；`handler/binlog_handler.go:52,75`（gopkg/pkg/errors fork `Errorf`，fundamental 无 Cause）与 `:85,88`（`Wrapf` 保留 Cause）；`rpc/redis.go:36,67`（github.com/pkg/errors v0.9.1 `Errorf`，其中 BatchSet 长度校验无调用方不可达）。
- 公共映射：handler 签名 `func(ctx,[]*events.CloudEvent)(*events.EventResponse,error)` 走 multi-cloudevent 路径；非 nil error → HTTP 500 + `X-Bytefaas-Response-Error-Code`/`X-Faas-Response-Error-Code`=`function_execution_error`（response_utils.go:50-55），`responseError` 写入的 Body 被框架在写 Body 前丢弃；成功路径 StatusCode=0 → `ValidateAndWriteStatusCode` 不写 → 默认 200 body `handle cloudevent success`。
- 框架错误码族（可由本服务触达）：function_execution_error(500)/function_panic(500)/invalid_cloud_event(400)/function_no_response(500)/invalid_event_type(400)/function_mismatch(409，function-id 头不匹配)/function_invalid_status_code(500，本服务恒 StatusCode=0 不可达)。
- 传播：binlog 两级 Unmarshal（dbrelay Message + gogo protobuf Entry）错误透传；未知表 default `return nil`；`DelKeys` 失败经 `delCache` 上抛触发 MQ 重试（代码注释明示）；`blockDelAction` 对 BatchGet（含 goredis 错误与 redis.go:67 nil 值）与 jsoniter Unmarshal 全部吞错返回 false（放过防漏删），仅 `binlogSubUser.UpdateAt <= cacheSubUser.UpdateAt` 拦删；KafkaMQEvent no-op、未知 ce.Type() 仅 CtxWarn。
- 底层族：goredis 哨兵族 ErrEmptyClusterName/ErrClusterConfigNotFound/ErrEmptyServerList/ErrConsulServerEmpty/ErrEtcdServerEmpty/ErrDegradated（error.go:5-13）；dbrelay `ConvertProtoToValue` NULL 列返回 (nil,nil) 不报错，报错分支为 strconv ParseInt/ParseFloat 失败与 `unknown column type[%s]`（parse.go:126-176）。

## 验证

归档完整副本（exact copy，非工作树）`GOFLAGS=-mod=vendor GOPROXY=off go build ./...` 退出码 0——本模块无 cgo 阻塞，可整模块构建；对照批次条目中 vendored cgo 包不编译的非阻塞口径。应用层 11 文件全量读取，非 grep 片段。

## Impact / 兼容

无 opencoder 代码影响。为该 repo 的 error-code 后续 verification 轮（需引用 `conclusion_sha256`）与同构 Go vendor 仓库分析提供事实基线与构建口径；沿用既有约定，暂不沉淀独立特性页（一次性 frozen 执行，非常态化巡检）。
