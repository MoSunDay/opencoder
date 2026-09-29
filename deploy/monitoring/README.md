# OpenCoder 调度监控

`register.py` 在现有 k3s `kube-system` 监控栈中注册 `opencoder-scheduler` 抓取任务和“OpenCoder 调度总览”Grafana 面板。运行前需先发布包含 `/metrics` 的 Server，在部署配置中设置独立的 `metrics_token_file`，并确认从 Prometheus Pod 可连接 `10.199.71.70:18081`。指标令牌只可访问 `GET /metrics`，必须与管理员令牌不同。

```bash
python3 deploy/monitoring/register.py --token-file /etc/opencoder/metrics.token
```

脚本先核对令牌只能访问 `/metrics`，并在首次改动前将监控 Deployment 的原配置写入权限为 0600 的 `/var/lib/opencoder/monitoring-rollback.json`。随后保留已有抓取任务和规则，生成新的不可变 Prometheus ConfigMap 并切换 Deployment；Grafana 通过单独 ConfigMap 挂载面板。原 ConfigMap 不会删除。Bearer 令牌来自运行机器上的文件，写入 Kubernetes Secret `opencoder-prometheus-scrape-token`，不进入 Git、命令行参数值或输出。重试不会覆盖原回滚记录。

要撤回本次监控注册，执行 `python3 deploy/monitoring/register.py --restore`。脚本只恢复原 Prometheus ConfigMap 引用并移除本次新挂载的令牌和面板来源；若 Prometheus 已改用其他配置，会拒绝覆盖。撤回后保留新增 ConfigMap 和 Secret，便于审计与再次注册。

指标令牌轮换后需重新运行注册脚本同步 Secret 并重启 Prometheus Pod 读取新凭据。监控栈日后切换 Prometheus ConfigMap 时，也需重新运行脚本接回此抓取任务。

抓取目标使用当前 OpenCoder 宿主机地址。宿主机地址或监控栈部署方式变化时，先修改 `prometheus-scrape.yml` 并检查网络可达性。Prometheus 运行状态可查询 `up{job="opencoder-scheduler"}`；Grafana 面板 UID 为 `opencoder-scheduler`，数据源 UID 为 `prometheus`。进程内计数器随 Server 重启归零，日程最近结果从持久化记录读取。

面板入口：<https://ai-viking.bytedance.net/grafana/d/opencoder-scheduler/opencoder-e8b083-e5baa6-e680bb-e8a788>。
