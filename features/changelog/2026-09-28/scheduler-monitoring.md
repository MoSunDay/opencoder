Commit: 6400d536e29baa88887115b02c6e0086af075dc5

# 调度指标接入监控平台

在既有 Server 调度指标上，k3s Prometheus 新增带凭据的 `opencoder-scheduler` 抓取任务，Grafana 通过文件配置加载 13 个“OpenCoder 调度总览”面板。面板将日程派发结果、实际执行结果与节点容量分开呈现。监控配置保留现有抓取任务和面板，可重复注册；凭据留在 Kubernetes Secret，不写入仓库。

## 测试覆盖

| 功能 | 测试名 | 文件 |
|------|--------|------|
| 独立指标凭据访问范围 | `test_only_metrics_get_is_allowed` | `deploy/monitoring/test_register.py` |
| 首次备份与重复注册 | `test_journal_keeps_first_anchor_across_retry` | `deploy/monitoring/test_register.py` |
| 定向恢复 | `test_restore_changes_only_registered_deployment_fields` | `deploy/monitoring/test_register.py` |

- 监控配置测试：`python3 -m unittest discover -s deploy/monitoring -p 'test_*.py'` → 3 passed / 0 failed。
- 全量回归与 clippy：见 [2026-09-29 收口记录](../2026-09-29/release-gate-closure.md)。

当前行为见[Agent 调度平台](../../agent-platform/index.md)，监控部署见[配置说明](../../../deploy/monitoring/README.md)。
