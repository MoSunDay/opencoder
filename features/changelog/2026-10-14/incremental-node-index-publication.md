Commit: 24ccca767038c83d23ed613eb7ddbec1c42a7486

# 完整索引同步期间保持控制连接响应

节点原来在连接循环中一次性写出整份索引和 Brain 报告；大报告连续即时写出或遭遇回压时，心跳、RPC 读取和回复都要等待整份报告。现在保留一个报告游标，在各批次之间调度心跳、RPC 和回复，并主动让出执行机会。快照与 Begin 相邻，Begin 固定原报告 sequence，后续负载快照不污染原报告水位。已有协议、帧顺序、执行归属、128 项队列和 20 秒输入期限保持。

功能 → 定向回归：

- 大报告回压时仍回复 RPC、发送心跳、保持所有索引及 Brain 帧顺序 → `backpressured_publication_services_rpc_and_heartbeat_before_report_end`。
- 报告中间的新负载快照不抬高旧报告水位 → `interleaved_load_snapshot_does_not_replace_report_watermark`。
- 读取失败、阻塞准入、断线取消所属清单读取 → `fleet::client::tests`。
- 慢数据库、迟到报告与同代次重连 → `node_keepalive::report_storage`。
- 无输入而持续发送、满队列、原 owner 保留后重连 → `transport::socket_io::tests`。
- 真实容器取消、进程树回收及持久 stop → `operations::admission::tests::stop_signal`。

94 版本在 79 的 900 秒、151 次持续执行及双端公共入口观察通过；本节点第 30 次观察返回 `503 node initial index sync pending`，这次失败保留，不能以公共入口仍有其他就绪节点替代本节点验收。后续发布需独立验证两端持续执行与实际控制通道恢复。
