# OpenCoder 调度监控

Server 的 `GET /metrics` 提供调度指标。使用独立的 `metrics_token_file` 配置只读指标凭证；管理员凭证与指标凭证必须不同。

将 `prometheus-scrape.yml` 中的地址改成部署环境的 Server 地址，把指标凭证文件放到 Prometheus 可读取的位置，然后将此抓取任务加入现有 Prometheus 配置。`opencoder-scheduler-dashboard.json` 可导入 Grafana，数据源 UID 为 `prometheus`。

进程内计数器随 Server 重启归零，日程最近结果从持久化记录读取。通过 `up{job="opencoder-scheduler"}` 检查抓取状态。
