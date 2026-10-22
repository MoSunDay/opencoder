# 发布兼容性定向验收

在本机隔离目录启动真实旧 Server、候选 Server 和 Runtime；不操作生产服务、生产数据库或 systemd unit。

```sh
python3 scripts/acceptance/release_compatibility/main.py \
  --old /path/to/pre-catalog-bundle \
  --bridge /path/to/report-reader-bundle \
  --candidate /path/to/report-writer-bundle \
  --legacy-controller /path/to/legacy/scripts/platform/rolling/manifest.py \
  --output /tmp/release-compatibility-evidence
```

旧包须使用 Turso、尚未增加目录表且 `report_state` 为 0/0；过渡包为 1/0，候选为格式 12、1/1。旧控制器源文件须保留原相对路径及其 `install_bundle.py` 依赖。

验收包含：旧连接持有事务时候选缺表启动被拒绝，旧 HTTP 和数据库连接继续成功；停止旧进程并验证备份后迁移；开放写入前恢复旧 Ontology 库；开放后拒绝旧备份恢复；完整结构的普通重启；writer 确认记录保存在独立 ACK 表且原 journal 字节不变，reader 重启后不重放已确认消息。

`receipt.json` 保存包摘要、兼容拒绝原因和各场景结果。故障时保留过程日志，清理仅限脚本自己启动的进程。该脚本不替代真实容器任务的交接回归或生产发布后的观察。
