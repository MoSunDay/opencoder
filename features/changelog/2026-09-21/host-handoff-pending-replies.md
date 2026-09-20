# Host 交接保留旧连接的在途回执

真实三版本发布与回滚验收中，旧 Host 的索引报告落盘期间，新 Host 可以完成接管。旧报告随后被当成连接错误，提前关闭仍承载请求回执的旧通道；旧通道的断开处理又因当前连接已换代而直接返回，使请求一直等待 RPC 超时。现场任务已经执行成功，但客户端等待 30 秒仍拿不到回执。

将报告收尾抽到独立模块：报告过期只停止其后续索引和容量确认，旧通道继续接收已发出请求的回执。连接断开始终按节点和连接代次结清该通道的等待者，同时保留新连接的在线状态和请求。持久化报告的代次隔离、原请求身份、错误报告和接收超时门槛均保留。

## 测试覆盖

| 功能 | 测试名 | 文件 |
| --- | --- | --- |
| 旧报告跨越新 Host 接管仍可收到原回执 | `superseded_report_keeps_the_old_reply_deliverable` | `crates/control/src/transport/handoff_report.rs` |
| 旧连接断开只结清自己的请求 | `superseded_disconnect_settles_only_its_own_calls` | 同上 |

修正前两项回归均失败：第一项报告过期错误，第二项请求超时。原始日志：`/var/tmp/opencoder-latest-release-20260920/handoff-reply-before.log`。对应真实隔离失败保留在 `/var/tmp/opencoder-latest-release-20260920/opencoder-smooth-f2o_59ej`。最终全量验证和上线结果另记发布回执。
